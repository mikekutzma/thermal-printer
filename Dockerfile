# syntax=docker/dockerfile:1
#
# Multi-stage build for thermal-printer-server.
#
# rusb uses --features vendored, so libusb is compiled from source and
# statically linked into the binary. The runtime image needs no libusb.
#
# Build for Raspberry Pi 4B (arm64):
#   docker buildx build --platform linux/arm64 -t thermal-printer-server .
#
# Build locally (native arch):
#   docker build -t thermal-printer-server .

# ── Stage 1: cargo-chef (pre-built, avoids compiling it from source) ─────────
FROM lukemathwalker/cargo-chef:latest-rust-slim-bookworm AS chef
WORKDIR /app

# ── Stage 2: compute the dependency recipe ───────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Stage 3: build dependencies (cached as long as Cargo.toml/lock unchanged) ─
FROM chef AS builder

# vendored libusb builds from source — needs cmake and a C toolchain
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p thermal-printer-server --recipe-path recipe.json

# ── Stage 4: build the application ───────────────────────────────────────────
COPY . .
RUN cargo build --release -p thermal-printer-server

# ── Stage 5: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/thermal-printer-server /usr/local/bin/

EXPOSE 3000
CMD ["thermal-printer-server"]
