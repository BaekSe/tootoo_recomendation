# syntax=docker/dockerfile:1

FROM rust:1.85-slim-bookworm AS builder

WORKDIR /app

# System deps for building (openssl vendored still needs some build tooling)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    pkg-config \
    build-essential \
  && rm -rf /var/lib/apt/lists/*

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/api/Cargo.toml crates/api/Cargo.toml
COPY crates/worker/Cargo.toml crates/worker/Cargo.toml

# Dummy sources to build deps layer
RUN mkdir -p crates/core/src crates/api/src crates/worker/src \
  && printf 'fn main() {}\n' > crates/api/src/main.rs \
  && printf 'pub fn _dummy() {}\n' > crates/core/src/lib.rs \
  && printf 'fn main() {}\n' > crates/worker/src/main.rs

RUN cargo build -p tootoo_api --release

# Real sources
COPY crates ./crates

RUN cargo build -p tootoo_api --release


FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/tootoo_api /app/tootoo_api

ENV PORT=3000
EXPOSE 3000

CMD ["/app/tootoo_api"]
