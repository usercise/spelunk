# spelunk-server — minimal production image
#
# Multi-stage build: compile in a Rust builder, copy the binary into a slim
# Debian image. The result is a ~50 MB image with no Rust toolchain overhead.
#
# Build:
#   docker build -t spelunk-server .
#
# Run (dev, no auth):
#   docker run -p 7777:7777 -v spelunk-data:/data spelunk-server
#
# Run (production, with API key):
#   docker run -p 7777:7777 -v spelunk-data:/data \
#     -e SPELUNK_SERVER_KEY=your-key \
#     spelunk-server

# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM rust:1.94.1-slim AS builder

WORKDIR /build

# Cache dependency compilation separately from source changes.
COPY Cargo.toml Cargo.lock ./
COPY src/lib.rs src/lib.rs
# Placeholder main.rs so the dependency step compiles.
RUN mkdir -p src/bin && \
    echo 'fn main(){}' > src/main.rs && \
    echo 'fn main(){}' > src/bin/spelunk_server.rs && \
    cargo build --release --bin spelunk-server 2>/dev/null || true

# Now copy the real source and build properly.
COPY . .
RUN touch src/bin/spelunk_server.rs && \
    cargo build --release --bin spelunk-server

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false spelunk
WORKDIR /data
RUN chown spelunk:spelunk /data

COPY --from=builder /build/target/release/spelunk-server /usr/local/bin/spelunk-server

USER spelunk

EXPOSE 7777

ENTRYPOINT ["/usr/local/bin/spelunk-server"]
CMD ["--db", "/data/spelunk.db"]
