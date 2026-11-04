# syntax=docker/dockerfile:1.7

# ---------------------------------------------------------------------------
# Build stage: compile the Fastcart server binary from the workspace.
# ---------------------------------------------------------------------------
FROM rust:1.97.1-bookworm AS builder

WORKDIR /app

# Copy only the workspace manifests and the crates needed by the server first.
COPY Cargo.toml Cargo.lock ./
COPY apps ./apps
COPY crates ./crates
COPY migrations ./migrations

# `--locked` keeps the image reproducible; the lockfile is part of the build
# context. The server binary is the only release artifact this image ships.
RUN cargo build --release --locked -p fastcart-server

# ---------------------------------------------------------------------------
# Runtime stage: minimal Debian image running as an unprivileged user.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        libsqlite3-0 \
        tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --system --gid 10001 fastcart \
    && useradd --system --uid 10001 --gid fastcart --home-dir /app --shell /usr/sbin/nologin fastcart

WORKDIR /app

COPY --from=builder /app/target/release/fastcart-server /usr/local/bin/fastcart-server

# The default configuration writes platform/store databases, assets and
# backups beneath ./data. Keep that directory writable by the runtime user so a
# named or bind-mounted volume inherits the ownership.
RUN mkdir -p /app/data/stores /app/data/assets /app/data/backups \
    && chown -R fastcart:fastcart /app

# Safe defaults for a container deployment. Secrets are never baked in; supply
# them at runtime (Compose, Docker secrets or your orchestrator).
ENV FASTCART_SERVER_HOST=0.0.0.0 \
    FASTCART_SERVER_PORT=8080 \
    FASTCART_LOG_LEVEL=info \
    FASTCART_STOREFRONT_ENABLED=false \
    RUST_LOG=info

EXPOSE 8080
VOLUME ["/app/data"]

USER fastcart:fastcart

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD-SHELL curl -fsS "http://127.0.0.1:${FASTCART_SERVER_PORT:-8080}/health/live" || exit 1

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/local/bin/fastcart-server"]
