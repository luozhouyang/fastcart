# syntax=docker/dockerfile:1.7

# ---------------------------------------------------------------------------
# Web build stage: build the bundled theme and the Admin SPA.
#
# Both are pnpm workspace packages. The theme bundle is copied into the server
# runtime below; the Admin bundle is built here so the workspace's frontend
# build is validated alongside the server, and is shipped by apps/admin's own
# nginx image. Neither `node_modules` nor the Node toolchain reaches the
# runtime image.
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS web-builder

ENV CI=true \
    COREPACK_ENABLE_DOWNLOAD_PROMPT=0

WORKDIR /app

# Keep the pnpm version in lockstep with the `packageManager` field in
# package.json. Corepack downloads it from npm during the build.
RUN corepack enable \
    && corepack prepare pnpm@10.33.0 --activate

# Copy the workspace manifests and sources needed by the theme and Admin
# builds. `.dockerignore` removes node_modules, dist and test artifacts, so
# every frontend artifact is rebuilt from source inside the image.
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc tsconfig.base.json ./
COPY apps ./apps
COPY packages ./packages
COPY themes ./themes

RUN pnpm install --frozen-lockfile
RUN pnpm --filter @fastcart/theme-default build \
    && pnpm --filter @fastcart/admin build

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

# Ship the default theme bundle at <themes_dir>/default. The server's default
# theme bootstrap looks for `theme.json` and the bundle manifests here, while
# the sidecar renders `dist/server/server.js` and the API serves
# `dist/client/*`. The files stay root-owned and read-only to the runtime
# user; only ./data is writable.
COPY --from=web-builder /app/themes/default/dist /app/themes/default/dist
COPY --from=web-builder \
    /app/themes/default/theme.json \
    /app/themes/default/assets.json \
    /app/themes/default/editor.manifest.json \
    /app/themes/default/settings.schema.json \
    /app/themes/default/

# The default configuration writes platform/store databases, assets and
# backups beneath ./data. Keep that directory writable by the runtime user so a
# named or bind-mounted volume inherits the ownership.
RUN mkdir -p /app/data/stores /app/data/assets /app/data/backups \
    && chown -R fastcart:fastcart /app/data

# Safe defaults for a container deployment. Secrets are never baked in; supply
# them at runtime (Compose, Docker secrets or your orchestrator).
ENV FASTCART_SERVER_HOST=0.0.0.0 \
    FASTCART_SERVER_PORT=8080 \
    FASTCART_LOG_LEVEL=info \
    FASTCART_STOREFRONT_ENABLED=false \
    FASTCART_THEMES_DIR=/app/themes \
    RUST_LOG=info

EXPOSE 8080
VOLUME ["/app/data"]

USER fastcart:fastcart

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${FASTCART_SERVER_PORT:-8080}/health/live" || exit 1

STOPSIGNAL SIGTERM

ENTRYPOINT ["/usr/local/bin/fastcart-server"]
