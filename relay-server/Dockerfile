FROM rust:1.82-slim AS builder
WORKDIR /app
COPY Cargo.toml .
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/relay-server /usr/local/bin/relay-server
EXPOSE 4000
CMD ["relay-server"]
