FROM rust:1.67 as builder
WORKDIR /usr/src/guestbook
COPY . .
RUN cargo build --release

FROM debian:bullseye-slim
WORKDIR /usr/src/guestbook
COPY --from=builder /usr/src/guestbook/target/release/guestbook ./guestbook
RUN apt-get update && apt-get install -y libsqlite3-0&& rm -rf /var/lib/apt/lists/*
CMD ["./guestbook"]

