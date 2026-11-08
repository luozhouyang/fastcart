# syntax=docker/dockerfile:1.7

# ---------------------------------------------------------------------------
# Theme build stage: the sidecar needs the compiled default theme bundle.
#
# The sidecar is built from the repository root context (see
# docker-compose.yml) because the bundle is a pnpm workspace package. Building
# the theme here makes the sidecar image self-contained: it does not depend on
# a local `pnpm build` having run before `docker build`, and the same source
# revision always produces the same bundle. The server image builds the theme
# too; BuildKit caches the identical install/build layers where possible.
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS theme-builder

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
RUN pnpm --filter @fastcart/theme-default build

# ---------------------------------------------------------------------------
# Runtime dependency stage: React for the compiled theme server bundle.
#
# Vite externalizes React for SSR, so themes/default/dist/server/server.js
# imports `react`, `react/jsx-runtime` and `react-dom/server.browser`. Keep
# these versions in sync with themes/default/package.json; pnpm-lock.yaml
# resolves both to 19.3.1. npm installs react-dom's `scheduler` dependency too.
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS runtime-deps

WORKDIR /deps

RUN npm install --omit=dev --no-audit --no-fund --ignore-scripts \
    react@19.3.1 \
    react-dom@19.3.1

# ---------------------------------------------------------------------------
# Runtime stage: Node 22, unprivileged, scripts plus the built theme bundle.
#
# The sidecar has no build tooling in this stage. It ships only the ESM scripts,
# the theme output and the small React runtime required by that output; the
# pnpm store and the rest of node_modules stay in the build stages.
# ---------------------------------------------------------------------------
FROM node:22-bookworm-slim AS runtime

ENV NODE_ENV=production \
    HOST=0.0.0.0 \
    PORT=8788 \
    THEME_ROOT=/app/themes

WORKDIR /app

COPY --chown=node:node apps/theme-sidecar/package.json ./package.json
COPY --chown=node:node apps/theme-sidecar/scripts ./scripts
COPY --from=runtime-deps --chown=node:node /deps/node_modules ./node_modules

# The server registers the bundled theme with bundle_path "default" under
# FASTCART_THEMES_DIR=/app/themes, so the sidecar resolves the server entry at
# /app/themes/default/dist/server/server.js. Keep the manifests beside dist so
# the bundle is complete for theme-asset requests as well.
COPY --from=theme-builder --chown=node:node \
    /app/themes/default/dist \
    /app/themes/default/dist
COPY --from=theme-builder --chown=node:node \
    /app/themes/default/theme.json \
    /app/themes/default/assets.json \
    /app/themes/default/editor.manifest.json \
    /app/themes/default/settings.schema.json \
    /app/themes/default/

USER node

EXPOSE 8788

# /health is always unauthenticated and does not invoke the theme worker.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:' + (process.env.PORT || '8788') + '/health').then((response) => process.exit(response.ok ? 0 : 1)).catch(() => process.exit(1))"

# SIDECAR_SECRET is intentionally not set in the image. A non-loopback bind
# without it is rejected by the sidecar's bind guard; fail fast here with a
# clear message instead of starting an unauthenticated renderer.
ENTRYPOINT ["sh", "-c", ": \"${SIDECAR_SECRET:?SIDECAR_SECRET must be set}\" && exec node scripts/serve.mjs"]

STOPSIGNAL SIGTERM
