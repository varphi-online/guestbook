use ascii::*;
use maud::*;
use sqlite::*;
use std::fs::*;
use std::path::*;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, SystemTime};
use tiny_http::*;
use urlencoding::decode;

fn main() {
    println!("Initializing server . . .");
    let addr = "127.0.0.1:8080";
    let server = Arc::new(tiny_http::Server::http(addr).unwrap());
    let sqlite = Arc::new(Mutex::new(
        Connection::open_thread_safe(Path::new("data/entries.db")).unwrap(),
    ));

    let query = "
        CREATE TABLE IF NOT EXISTS entries (name TEXT, domain TEXT, message TEXT, color TEXT, time INTEGER, public BOOLEAN);
    ";

    sqlite.lock().unwrap().execute(query).unwrap();
    println!("Now listening on {}", addr);

    let shutdown_signal = Arc::new(AtomicBool::new(false));
    let stopped_threads = Arc::new(AtomicUsize::new(0));

    const NUM_THREADS: usize = 4;

    let signal = shutdown_signal.clone();
    let server_signal = server.clone();

    ctrlc::set_handler(move || {
        println!("\nReceived Ctrl+C! Initiating graceful shutdown...");
        signal.store(true, Ordering::SeqCst);
        // Unblock the server for each worker thread
        for _ in 0..NUM_THREADS {
            server_signal.unblock();
        }
    })
    .expect("Error setting Ctrl-C handler");

    let mut handles = Vec::new();

    //https://github.com/tiny-http/tiny-http/issues/146

    for thread_id in 0..NUM_THREADS {
        let server = server.clone();
        let sqlite = sqlite.clone();
        let shutdown_signal = shutdown_signal.clone();
        let stopped_threads = stopped_threads.clone();

        let thread_builder = thread::Builder::new();
        handles.push(
            thread_builder
                .spawn(move || {
                    println!("Worker thread {} initialized", thread_id);
                    for mut request in server.incoming_requests() {
                        println!("{:?}", request);
                        match request.method() {
                            Method::Get => {
                                match request.url() {
                                    "/entries" => get_entries(&sqlite, Some(request)),
                                    _ => file_route(request),
                                };
                            }
                            Method::Post => {
                                let mut content = String::new();
                                request.as_reader().read_to_string(&mut content).unwrap();
                                update_db(decode(content.as_str()).unwrap().to_string(), &sqlite);
                                let (html, header) = get_entries(&sqlite, None).unwrap();
                                let _ = request.respond(
                                    Response::from_string(html.into_string())
                                        .with_header(header)
                                        .with_status_code(StatusCode::from(201)),
                                );
                            }
                            _ => (),
                        }
                        if shutdown_signal.load(Ordering::Relaxed) {
                            println!("Thread {} stopping gracefully", thread_id);
                            break;
                        }
                    }
                    stopped_threads.fetch_add(1, Ordering::Relaxed);
                    println!("Thread {} stopped", thread_id);
                })
                .unwrap(),
        );
    }

    let shutdown_monitor = thread::Builder::new()
        .spawn(move || {
            println!("Shutdown monitor thread initialized");

            while !shutdown_signal.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(750));
            }

            println!("Waiting for threads to stop (timeout: 30 seconds)...");

            for _ in 0..30 {
                if stopped_threads.load(Ordering::Relaxed) >= NUM_THREADS {
                    println!("All threads stopped successfully");
                    return;
                }
                thread::sleep(Duration::from_secs(1));
            }

            println!("Shutdown timeout reached, forcing exit");
            std::process::exit(1);
        })
        .unwrap();

    for h in handles {
        h.join().unwrap();
    }

    shutdown_monitor.join().unwrap();

    println!("Server shutdown complete.");
}

fn get_entries(
    db: &Arc<Mutex<ConnectionThreadSafe>>,
    request: Option<Request>,
) -> Option<(Markup, Header)> {
    let query = "SELECT * FROM entries WHERE public == true";
    let rows: Vec<_> = db
        .lock()
        .unwrap()
        .prepare(query)
        .unwrap()
        .into_iter()
        .map(|row| row.unwrap())
        .collect();
    let out = (
        html! {
        #entries hx-swap-oob="true" {
            @for row in rows.into_iter().rev(){
                .entry {
                    .entry_name {
                        p
                            style=(format!("color: {};",row.read::<&str, _>("color"))) {
                                (row.read::<&str, _>("name"))
                        }
                        @let domain = row.read::<&str, _>("domain").strip_prefix("https://").unwrap_or("");
                        @if !domain.is_empty() {
                            a href=(format!("https://{}",domain)) target="_blank" style="color: lightgray;" {
                            span style="font-size: 0.7em; margin: 0px;" {"@"}(domain)}
                        }
                        p.time {(row.read::<i64, _>("time"))}
                    }
                    p.entry_message {(row.read::<&str, _>("message"))}
                }
            }
        }
        },
        Header {
            field: tiny_http::HeaderField::from_str("Content-type").unwrap(),
            value: AsciiString::from_str("text/css").unwrap(),
        },
    );
    match request {
        None => Some(out),
        Some(rq) => {
            let _ = rq.respond(Response::from_string(out.0.into_string()).with_header(out.1));
            None
        }
    }
}

fn update_db(content: String, database: &Arc<Mutex<ConnectionThreadSafe>>) {
    let color = validate_color(&content[6..13]);
    let name = &content[content.find("&name=").expect("No name id") + 6
        ..content.find("&domain=").expect("No domain id")];
    let mut domain = &content[content.find("&domain=").expect("No domain id") + 8
        ..content.find("&message=").expect("No message id")];
    let owned_domain: String;
    if !domain.starts_with("https://") {
        owned_domain = format!("https://{}", domain);
        domain = owned_domain.as_str();
    }
    let message = &content[content.find("&message=").expect("No message id") + 9..content.len()];
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let query = "INSERT INTO entries VALUES (:name,:domain,:message,:color,:time,true)";

    let db = database.lock().unwrap();
    let mut statement = db.prepare(query).unwrap();
    statement
        .bind_iter::<_, (_, Value)>([
            (":name", name.into()),
            (":domain", domain.into()),
            (":message", message.into()),
            (":color", color.into()),
            (":time", now.into()),
        ])
        .unwrap();
    statement.next().unwrap();
}

fn file_route(request: Request) -> Option<(Markup, Header)> {
    let path = Path::new(match request.url() {
        "/" => "index.html",
        _ => request
            .url()
            .strip_prefix("/")
            .unwrap_or("page_not_found.html"),
    });

    let file = File::open(path);
    if file.is_ok() {
        let response = tiny_http::Response::from_file(file.unwrap());

        let response = response.with_header(tiny_http::Header {
            field: "Content-Type".parse().unwrap(),
            value: AsciiString::from_ascii(get_content_type(path)).unwrap(),
        });
        let _ = request.respond(response);
        None
    } else {
        println!("File: {:?} not found!", path);
        let rep = tiny_http::Response::new_empty(tiny_http::StatusCode(404));
        let _ = request.respond(rep);
        None
    }
}

fn get_content_type(path: &Path) -> &'static str {
    let extension = match path.extension() {
        None => return "text/plain",
        Some(e) => e,
    };

    match extension.to_str().unwrap() {
        "otf" => "font/otf",
        "gif" => "image/gif",
        "jpg" => "image/jpeg",
        "jpeg" => "image/jpeg",
        "png" => "image/png",
        "pdf" => "application/pdf",
        "htm" => "text/html; charset=utf8",
        "html" => "text/html; charset=utf8",
        "js" => "text/javascript",
        "css" => "text/css",
        "txt" => "text/plain; charset=utf8",
        _ => "text/plain; charset=utf8",
    }
}

fn validate_color(input: &str) -> &str {
    if input.chars().next().unwrap_or(' ') != '#' {
        return "#000000";
    };
    let valid = "0123456789abcdef";
    for c in input[1..].chars() {
        if !valid.contains(c) {
            return "#000000";
        }
    }
    input
}
