# Fastcart

A lightweight, self-hosted, multi-store commerce platform built with Rust and React.

Fastcart aims to deliver the developer experience of modern headless commerce platforms while keeping deployment simple: a single Rust modular monolith, one SQLite database per store, a React admin application, and an embedded storefront runtime.

> **Status: broad Rust backend, a narrow but functional Admin, and a storefront shell.**
> The Rust platform, commerce modules, workflows, HTTP APIs, theme registry,
> webhook and email delivery, observability and OpenAPI document are implemented
> and covered by unit and integration tests. Typed TypeScript SDKs provide the
> front-end client surface. The React Admin is a real application covering
> orders, catalog draft create/list/export, inventory, customers, content,
> themes, forms, inbox, notifications and settings, but it has no price-authoring,
> promotion, region, shipping or tax screens even though the backend exposes
> those operations. `apps/storefront` is a non-production development harness.
> Storefront SSR is wired through the Node sidecar; the embedded V8 adapter is
> still a stub, and selecting it disables SSR.

## Why Fastcart

- **Simple to deploy.** Single-node, single-binary first. No mandatory Redis, PostgreSQL or message broker.
- **Multi-store by design.** Every store owns an isolated SQLite database; store context is resolved per request from trusted platform metadata.
- **Rust performance.** Async Rust with Axum and Tokio, integer money, explicit module boundaries.
- **Medusa-style modules.** Catalog, pricing, inventory, customer, cart, promotion, checkout, order, payment, shipping, tax, content and webhooks are independent crates; workflows compose them.
- **Extensible core.** Provider interfaces, capability-scoped plugin manifests, a transactional outbox, a persistent job queue, and an auditable AI tool registry.

## Implemented backend

### Platform

- Store registry, lifecycle (`provisioning` / `active` / `suspended` / `deleting` / `deleted`) and idempotent provisioning.
- Domain registry with normalization, primary-domain rules and duplicate detection.
- Platform users, Argon2id credentials, server-side admin sessions, CSRF-protected cookie auth and hashed API tokens.
- Roles and permissions: `products.*`, `orders.*`, `customers.*`, `inventory.*`, `promotions.*`, `settings.*`, `themes.*`, `users.*`, `payments.*`.
- Audit log, per-store SQLite pools, migrations, and a consistent `VACUUM INTO` backup path for destructive store deletion; `scripts/verify-backup.sh` verifies a backup file's integrity and schema. The CLI also exposes `backup`, `verify` and `restore` commands (`crates/cli/src/main.rs`).

### Commerce modules

| Module    | Capabilities                                                                                     |
| --------- | ------------------------------------------------------------------------------------------------ |
| store     | Store settings and sales channels                                                                |
| catalog   | Products, variants, options, media, categories, collections, tags, FTS5 search, publication      |
| pricing   | Price lists, scoped prices and deterministic price selection                                     |
| promotion | Percentage/fixed/free-shipping promotions, coupons, rules, usage limits, proportional allocation |
| inventory | Locations, levels, atomic reservations, release/commit, expiry and movement history              |
| shipping  | Profiles, options and region/weight-aware flat rate selection                                    |
| tax       | Categories, regions, rates and inclusive/exclusive tax calculation                               |
| customer  | Profiles, addresses, groups, Argon2id credentials and store-scoped sessions                      |
| cart      | Snapshot line items and server-derived totals                                                    |
| checkout  | Persisted state machine with transition history                                                  |
| order     | Immutable commercial snapshots, idempotent placement, fulfillment and refunds                    |
| payment   | Provider registry, sandbox provider with HMAC webhooks, capture, refunds, idempotent events      |
| content   | Pages, versions, navigation trees and redirects                                                  |
| webhook   | Endpoint subscriptions, HMAC-signed deliveries, retries, replay and delivery history             |
| theme     | Theme registry, JSON Schema settings, activation, rollback and public active-theme lookup        |

### Cross-cutting

- Transactional outbox with retry/backoff and in-process event bus.
- SQLite-backed persistent job queue with atomic claiming and lease expiry.
- Signed webhook delivery from outbox events with idempotent fan-out, retry/backoff, crash recovery and replay.
- Provider-independent transactional email with queued `send_email` jobs, SMTP (`lettre`) and Resend (`resend-rs`) providers, plus log and in-memory test providers.
- Storefront SSR through the Node sidecar: `worker_count` creates that many HTTP clients behind a Rust semaphore with `queue_capacity` in-flight slots and fail-fast rejection, while the sidecar runs each render in its own worker thread with heap and wall-clock limits. The sidecar itself has no global concurrency cap, and the worker cap is not CPU isolation. Active theme context and trusted store resolution are applied before rendering.
- Periodic maintenance: cart, reservation and checkout expiry; outbox dispatch; webhook delivery; email jobs; simple job handlers.
- Store-scoped Moka cache with generation-based invalidation.
- Prometheus-compatible `/metrics` with latency histograms, plus baseline security response headers.
- OpenAPI 3.1 schemas generated from the Rust response types with `utoipa`, with a cached document and schema-reference validation.
- Local object storage with path-traversal protection.
- Plugin manifest registry with capability checks and dependency resolution.
- AI provider/model/tool registries with an auditable agent-task state machine.

## Repository layout

| Path                         | Purpose                                                          |
| ---------------------------- | ---------------------------------------------------------------- |
| `apps/server`                | Axum server bootstrap, configuration, graceful shutdown          |
| `apps/admin`                 | React Admin application for the implemented store operations     |
| `apps/storefront`            | TanStack Start harness (non-production; see README)              |
| `crates/kernel`              | IDs, money, contexts, validation, errors                         |
| `crates/platform`            | Platform DB, stores, domains, auth, RBAC, provisioning           |
| `crates/infrastructure`      | SQLite pools, migrations, cache, outbox, jobs, storage           |
| `crates/http`                | Error envelope, extractors, auth/store middleware, rate limiting |
| `crates/modules/*`           | The independent commerce modules listed above                    |
| `crates/workflows`           | Cart pricing, checkout and maintenance workflows                 |
| `crates/api`                 | Versioned HTTP surface and DTOs                                  |
| `crates/plugin`, `crates/ai` | Extension foundations                                            |
| `migrations/platform`        | Platform schema                                                  |
| `migrations/store`           | Per-store schema (catalog, pricing, inventory, orders, …)        |

## Getting started

```bash
cargo build --workspace
cargo run -p fastcart-cli -- init --dir .
cargo run -p fastcart-cli -- user create --email owner@example.com --password 'change-me-please'
cargo run -p fastcart-cli -- store create \
  --name "Demo Store" --slug demo --domain shop.example.com \
  --owner-email owner@example.com
cargo run -p fastcart-cli -- serve
```

Alternatively, start the server first and bootstrap through `POST /api/v1/platform/auth/setup`; it only succeeds while no users exist.

### Configuration

`config/config.toml` is optional; every field has a default. Environment overrides:

- `FASTCART_CONFIG` — configuration path
- `FASTCART_SERVER_HOST`, `FASTCART_SERVER_PORT`, `FASTCART_LOG_LEVEL`
- `FASTCART_PAYMENT_WEBHOOK_SECRET` — enables sandbox webhook verification
- `FASTCART_EMAIL_PROVIDER` — `log`, `smtp` or `resend`
- `FASTCART_EMAIL_FROM` — default sender address
- `FASTCART_SMTP_HOST`, `FASTCART_SMTP_PORT`, `FASTCART_SMTP_USERNAME`,
  `FASTCART_SMTP_PASSWORD`, `FASTCART_SMTP_SECURITY` — SMTP delivery
- `FASTCART_RESEND_API_KEY` — Resend delivery
- `FASTCART_STOREFRONT_ENABLED`, `FASTCART_STOREFRONT_RUNTIME` — enable SSR (default off) and select `node`/`v8`
- `FASTCART_STOREFRONT_STREAMING`, `FASTCART_STOREFRONT_STREAM_TIMEOUT_MS` — allow streamed HTML (default on) and its total time budget (default 30s)
- `FASTCART_STOREFRONT_NODE_URL`, `FASTCART_STOREFRONT_NODE_SECRET` — Node sidecar endpoint and bearer secret
- `FASTCART_THEMES_DIR` — trusted theme bundle directory (default `./themes`)
- `FASTCART_INTERNAL_FETCH_SECRET` — mounts the secret-gated `/internal/storefront` HTTP surface
- `FASTCART_CAPABILITY_SECRET` — signs theme capability grants so plugin endpoints can verify them

Storefront SSR uses the Node sidecar (`runtime = "node"`, the default runtime).
The embedded `v8` provider is not built yet; starting with `runtime = "v8"`
logs a warning and serves API routes without SSR.

Build the bundled theme and start the sidecar before enabling storefront SSR:

```bash
pnpm --filter @fastcart/theme-default build
SIDECAR_SECRET=change-me pnpm --filter @fastcart/theme-sidecar start
```

The sidecar listens on `127.0.0.1:8788` by default; set `HOST=0.0.0.0` in a
container and always pair a non-loopback bind with `SIDECAR_SECRET`. New stores
get the default theme registered and activated automatically when the bundle is
present under `storefront.themes_dir`.

Themes may return a `ReadableStream` or async iterable from `render`. When
`streaming_enabled` is on, the sidecar forwards the theme's status and headers
and then the HTML bytes as they are produced, marking the response with
`x-fastcart-mode: stream`. The bundled default theme uses
`renderToReadableStream`, so the document head, styles and shell reach the
browser before the page tree finishes. Buffered themes and older hosts keep the
existing JSON contract. Streaming requires the reverse proxy to pass chunked
responses through without buffering.

### Theme trust model

Theme bundles are **trusted server-side code**. Registering and activating a
theme is a privileged operation: the sidecar imports the bundle in a worker
thread and the bundle can use Node APIs, including the filesystem and network.
The worker caps heap and wall-clock time; it does not cap CPU or provide an
operating-system sandbox. It is a reliability boundary, not a security boundary.

Run the sidecar with the same care as any application server: a non-root user,
a read-only root filesystem with only the theme directory writable during
deploys, no unnecessary network egress, and an OS/container security profile.
Keep the sidecar bound to loopback or behind `SIDECAR_SECRET`. If untrusted
third-party themes become a requirement, use a separate sidecar per trust
domain or wait for the embedded V8 runtime, which can restrict host access at
the isolate level.

Themes may also declare dynamic routes in `theme.json`, for example
`"routes": { "product": "/products/{handle}", "collection": "/collections/{handle}" }`.
A published page whose `page_type` matches the route kind is used as the
template; the host resolves the referenced record and exposes it to the theme as
`data.context.route`. The sitemap only advertises those URLs when the route and
a published template exist. Draft previews follow the same matching: the
preview path selects the handle, an optional `preview_handle` query parameter
overrides it, and otherwise the first published record of the template's kind
is used. Sitemap entries carry the record's UTC update time as `<lastmod>`;
`changefreq`/`priority` are omitted because search engines ignore them, and
`hreflang` alternates require a translation model the platform does not yet
have.

Theme editor contract: `editor.manifest.json` is the stable, data-only contract
used by the admin palette, inspector and server-side page validation.
`editor.js` is an optional executable component registry for hosts that want to
render theme components themselves. Theme bundles inline their own React copy,
so `editor.js` must run in an isolated document such as a sandboxed iframe; it
must not be imported into another React application's tree.

Local development uses `cookie_secure = false` and explicit `cors_origins` in `config/config.example.toml`.

## Deployment (Docker Compose)

`docker-compose.yml` runs the production-shaped stack as three containers:

| Service         | Image                    | Role                                                                          | Published port             |
| --------------- | ------------------------ | ----------------------------------------------------------------------------- | -------------------------- |
| `fastcart`      | `fastcart/server`        | Rust API and storefront host; SQLite data lives in the `data` volume          | `127.0.0.1:8080`           |
| `theme-sidecar` | `fastcart/theme-sidecar` | Node 22 renderer for the bundled theme; internal-only render and asset server | none (`8788` in the stack) |
| `admin`         | `fastcart/admin`         | nginx serving the Admin SPA and proxying `/api/` to `fastcart:8080`           | `127.0.0.1:8081`           |

The server and sidecar images both build `themes/default` from source and bake
the bundle in; rebuild those two images after changing the theme. The Admin
image contains only `apps/admin/dist` and nginx, not `node_modules`. The server
image sets `FASTCART_THEMES_DIR=/app/themes`, and the sidecar resolves the
auto-registered default bundle at `/app/themes/default`.

Create secrets, then build and start the stack:

```bash
export SIDECAR_SECRET="$(openssl rand -base64 48)"
export FASTCART_PREVIEW_SECRET="$(openssl rand -base64 48)"
export FASTCART_CAPABILITY_SECRET="$(openssl rand -base64 48)"
export FASTCART_COOKIE_SECURE=true   # when served over HTTPS
docker compose build
docker compose up -d
```

Open the Admin at <http://127.0.0.1:8081>. For local HTTP-only testing set
`FASTCART_COOKIE_SECURE=false`; otherwise the browser will not send the secure
session cookie over plain HTTP.

Required and recommended environment:

- `SIDECAR_SECRET` — long random bearer token. Compose passes it to the sidecar
  as `SIDECAR_SECRET` and to the server as `FASTCART_STOREFRONT_NODE_SECRET`, so
  both sides always match. Compose refuses to render without it.
- `FASTCART_STOREFRONT_NODE_SECRET` — the server-side copy of the sidecar
  secret. Set it to the same value when running the containers without Compose;
  Compose derives it from `SIDECAR_SECRET`.
- `FASTCART_PREVIEW_SECRET` — at least 32 bytes; enables draft preview. Without
  it the preview endpoints return `501 Not Implemented`.
- `FASTCART_CAPABILITY_SECRET` — signs theme capability grants. Set a long
  random value in production when themes use capability endpoints.
- `FASTCART_INTERNAL_FETCH_SECRET` — shared secret that mounts the
  secret-gated `/internal/storefront` surface; needed when a theme or sidecar
  calls that boundary.
- `FASTCART_COOKIE_SECURE=true` — required behind TLS so session and CSRF
  cookies are marked `Secure`.
- `FASTCART_STORE_<STORE>_STRIPE_SECRET_KEY`,
  `FASTCART_STORE_<STORE>_STRIPE_WEBHOOK_SECRET` and
  `FASTCART_STORE_<STORE>_MANUAL_PAYMENT_SECRET` — optional per-store payment
  provider overrides. `<STORE>` is the store ID upper-cased with every
  non-alphanumeric byte replaced by `_` (for example
  `store_01abc` → `STORE_01ABC`). Stores without overrides use the global
  `FASTCART_STRIPE_*` / `FASTCART_PAYMENT_WEBHOOK_SECRET` values.
- `FASTCART_STORE_<STORE>_EMAIL_PROVIDER`, `_EMAIL_FROM`, `_SMTP_HOST`,
  `_SMTP_PORT`, `_SMTP_USERNAME`, `_SMTP_PASSWORD`, `_SMTP_SECURITY` and
  `_RESEND_API_KEY` — optional per-store transactional-email overrides. They
  take effect when a queued email is delivered, so a restart is enough to pick
  up changed credentials. Stores without overrides inherit the global email
  configuration.
- `FASTCART_TRUSTED_PROXIES` — comma-separated IP addresses/CIDRs whose
  `X-Forwarded-For` chain the server may trust (for example the external TLS
  proxy). Empty means forwarding headers are ignored and the socket peer is
  used; this prevents clients from rotating `X-Forwarded-For` to bypass
  cart/checkout/inquiry limits. The Admin nginx container proxies only `/api/`
  for the Admin, not storefront writes, so it normally does not need to be
  listed.
- `FASTCART_PREVIEW_FRAME_ANCESTORS` — the Admin origin allowed to embed the
  preview iframe, for example `https://admin.example.com`.
- `FASTCART_ADMIN_BASE_URL` — absolute Admin URL used in notification deep
  links; defaults to `http://localhost:8081` in Compose.

The Admin container is the browser origin: nginx serves the SPA, and `/api/`
requests are proxied to the server with `Host` set to the Admin host and
`X-Real-IP`, `X-Forwarded-For`, `X-Forwarded-Host` and `X-Forwarded-Proto`
forwarded. This keeps session and CSRF cookies same-origin, so the Admin needs
no CORS configuration. TLS termination is external: put a reverse proxy in
front of the loopback Admin port (`8081`) and the loopback server port (`8080`)
for storefront hosts, and forward `X-Forwarded-Proto` from that proxy. The Admin
nginx config forwards the incoming scheme to the server.

Health endpoints: the server exposes `/health/live` (used by its container
healthcheck), the sidecar exposes `/health`, and the Admin nginx container
exposes `/healthz`. `docker compose up` waits for the sidecar before starting the
server and for the server before starting the Admin. The `data` named volume
persists `/app/data`; removing containers does not remove it.

## HTTP API

All endpoints are under `/api/v1` unless noted.

| Namespace                  | Purpose                                          |
| -------------------------- | ------------------------------------------------ |
| `/platform/*`              | Installation, auth, stores, domains, memberships |
| `/admin/*`                 | Store-scoped administration                      |
| `/store/*`                 | Public storefront and customer operations        |
| `/webhooks/payments/*`     | Provider callbacks                               |
| `/internal/storefront`     | Trusted SSR-to-commerce boundary                 |
| `/health`, `/health/ready` | Liveness and readiness                           |
| `/metrics`                 | Prometheus-compatible process metrics            |
| `/api/v1/openapi.json`     | OpenAPI 3.1 document                             |
| `/*`                       | Storefront SSR fallback on store hosts           |

Key flows:

```text
POST /api/v1/platform/auth/setup
POST /api/v1/platform/stores
GET  /api/v1/admin/catalog/products          # x-store-id + admin session
POST /api/v1/store/carts
POST /api/v1/store/carts/{id}/items
POST /api/v1/store/carts/{id}/shipping-method
POST /api/v1/store/checkouts
POST /api/v1/store/checkouts/{id}/complete
```

### Security model

- The store is resolved from the request `Host` header against the platform registry; clients cannot select a store with a header or query parameter on storefront routes.
- Admin requests authenticate by HttpOnly session cookie or bearer API token and select a store with `x-store-id`; the store must be one the user belongs to.
- Cookie-authenticated mutations require the double-submit CSRF token in `x-csrf-token`.
- All money is integer minor units. Prices, discounts, tax and shipping are computed server-side; client-submitted totals are never trusted.
- Credentials and API/session tokens are hashed (Argon2id for passwords, SHA-256 for tokens) and never returned after creation.

## Testing and quality gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm install
pnpm lint && pnpm typecheck && pnpm test && pnpm build
```

The suite covers module rules, repository behaviour, platform auth/RBAC, cart and checkout workflows, storefront HTTP flows (catalog → cart → tax → checkout → order), cancellations and restocking, webhook signing/retries, SQLite write serialization under concurrency, and cross-store tenant isolation. The OpenAPI tests assert a 3.1.0 document, namespace coverage and resolvable schema references; there is no TypeScript code-generation or type-parity check in the repository.

## Documentation

The living project docs — constitution, v1 PRD, decision records and architecture views — are in
[`.livingdocs/`](.livingdocs/index.md). The design notes under `docs/` are local-only working
material and are intentionally not versioned.

## Roadmap

- React admin and storefront feature UIs.
- Embedded V8 runtime adapter.
- Additional payment, shipping, tax, email and search providers.
- PostgreSQL adapter for high-traffic stores.

## License

MIT
