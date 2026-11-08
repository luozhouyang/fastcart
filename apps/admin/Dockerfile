# syntax=docker/dockerfile:1.7

# ---------------------------------------------------------------------------
# Admin build stage: Vite produces a static SPA in apps/admin/dist.
#
# The build context is the repository root because the Admin consumes pnpm
# workspace packages. Only the build output is copied into the runtime image;
# node_modules and the pnpm store stay in this stage.
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS builder

ENV CI=true \
    COREPACK_ENABLE_DOWNLOAD_PROMPT=0

WORKDIR /app

RUN corepack enable \
    && corepack prepare pnpm@10.33.0 --activate

COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc tsconfig.base.json ./
COPY apps ./apps
COPY packages ./packages
COPY themes ./themes

RUN pnpm install --frozen-lockfile
RUN pnpm --filter @fastcart/admin build

# ---------------------------------------------------------------------------
# Runtime stage: nginx serves the SPA and proxies /api/ to the Rust server.
#
# Keeping the Admin on the same origin as the proxied API means the session
# and CSRF cookies work without cross-origin CORS configuration.
# ---------------------------------------------------------------------------
FROM nginx:alpine AS runtime

COPY deploy/admin-nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/apps/admin/dist /usr/share/nginx/html

EXPOSE 80

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD wget -q -O /dev/null http://127.0.0.1/healthz || exit 1

STOPSIGNAL SIGTERM
