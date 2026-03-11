# Stage 1: Build
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY rules/ rules/

RUN cargo build --release --package aequi-server

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aequi-server /usr/local/bin/aequi-server

RUN mkdir -p /data

ENV AEQUI_DB_PATH=/data/aequi.db
ENV AEQUI_PORT=8060

EXPOSE 8060
VOLUME /data

ENTRYPOINT ["aequi-server"]
