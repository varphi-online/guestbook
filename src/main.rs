use ascii::*;
use maud::*;
use sqlite::*;
use std::fs::*;
use std::path::*;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;
use urlencoding::decode;

use tiny_http::*;

fn main() {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:8080").unwrap());
    let sqlite = Arc::new(Mutex::new(
        Connection::open_thread_safe(Path::new("test.db")).unwrap(),
    ));

    let query = "
        CREATE TABLE IF NOT EXISTS entries (name TEXT, domain TEXT, message TEXT, color TEXT, time INTEGER, public BOOLEAN);
    ";

    sqlite.lock().unwrap().execute(query).unwrap();
    println!("Now listening on port 8080");

    let mut handles = Vec::new();

    for _ in 0..4 {
        let server = server.clone();
        let sqlite = sqlite.clone();

        handles.push(thread::spawn(move || {
            for mut request in server.incoming_requests() {
                println!("{:?}", request);
                match request.method() {
                    Method::Get => {
                        match request.url() {
                            "/test" => get_entries(&sqlite, Some(request)),
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
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
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
                            a href=(format!("https://{}",domain)) target="_blank" style="color: gray;" {"@"(domain)}
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
    println!("String: {}, is len {}", content, content.len());
    let color = &content[content.find("color=").expect("No color id") + 6
        ..content.find("&name=").expect("No name id")];
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
        "css" => "text/css",
        "txt" => "text/plain; charset=utf8",
        _ => "text/plain; charset=utf8",
    }
}
