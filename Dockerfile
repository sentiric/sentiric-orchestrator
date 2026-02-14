# --- Builder ---
FROM rust:1.93-slim-bookworm AS builder
RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN cargo build --release

# --- Final ---
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    make \
    && curl -fsSL https://get.docker.com | sh \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/sentiric-orchestrator .

# Standard Environment
ENV RUST_LOG=info
EXPOSE 11080 11081

ENTRYPOINT ["./sentiric-orchestrator"]