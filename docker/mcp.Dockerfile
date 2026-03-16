# syntax=docker/dockerfile:1

FROM rust:1.89-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin mcp-server

FROM debian:bookworm-slim
WORKDIR /app

COPY --from=builder /app/target/release/mcp-server /usr/local/bin/mcp-server

ENTRYPOINT ["/usr/local/bin/mcp-server"]
