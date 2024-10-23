FROM rust:1.67 as builder
WORKDIR /usr/src/guestbook
COPY . .

RUN cargo build --release 

# Start a fresh image
FROM debian:bullseye-slim

ENV RUST_BACKTRACE=1

WORKDIR /usr/src/guestbook

RUN apt-get update && apt-get install -y libsqlite3-0 && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /usr/src/guestbook/data

COPY --from=builder /usr/src/guestbook/target/release/guestbook ./guestbook
RUN chmod +x ./guestbook

COPY --from=builder /usr/src/guestbook/index.html ./
COPY --from=builder /usr/src/guestbook/index.css ./
COPY --from=builder /usr/src/guestbook/page_not_found.html ./
COPY --from=builder /usr/src/guestbook/W95FA.otf ./
COPY --from=builder /usr/src/guestbook/htmx.min.js ./

VOLUME ["/usr/src/guestbook/data"]

EXPOSE 8080

CMD ["./guestbook"]
