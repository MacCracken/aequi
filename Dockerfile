# Stage 1: Build
FROM rust:1.89-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY rules/ rules/

RUN cargo build --release --package aequi-server

# Stage 2: Runtime — AGNOS base OS
FROM ghcr.io/maccracken/agnosticos:latest

LABEL org.opencontainers.image.title="Aequi"
LABEL org.opencontainers.image.description="Accounting and financial management platform on AGNOS"
LABEL org.opencontainers.image.source="https://github.com/MacCracken/aequi"
LABEL org.opencontainers.image.base.name="ghcr.io/maccracken/agnosticos:latest"

COPY --from=builder /build/target/release/aequi-server /usr/local/bin/aequi-server

USER root
RUN groupadd -g 1006 aequi && useradd -u 1006 -g aequi -m -s /bin/bash aequi
RUN mkdir -p /data && chown aequi:aequi /data
USER aequi

ENV AEQUI_DB_PATH=/data/aequi.db
ENV AEQUI_PORT=8060

EXPOSE 8060
VOLUME /data

ENTRYPOINT ["aequi-server"]
