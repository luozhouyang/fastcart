# Fastcart

A lightweight, self-hosted, multi-store commerce platform built with Rust and React.

Fastcart aims to deliver the developer experience of modern headless commerce platforms while keeping deployment simple: a single Rust modular monolith, one SQLite database per store, a React admin application, and an embedded storefront runtime.

> **Status: backend implementation complete; UI is still a shell.**
> The platform, commerce modules, workflows, HTTP APIs, theme registry, webhook
> delivery, email delivery, observability, OpenAPI document and typed
> TypeScript SDKs are implemented and covered by unit and integration tests.
> The React admin/storefront applications are shells. Storefront SSR is wired
> through the Node sidecar runtime; the embedded V8 adapter remains future work.

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
- Audit log, per-store SQLite pools, migrations, consistent `VACUUM INTO` backups, integrity checks and restore.

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
- Storefront SSR rendering through a bounded Node sidecar pool, with active theme context, trusted store resolution and streamed HTML shells.
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
| `apps/admin`                 | React admin shell                                                |
| `apps/storefront`            | TanStack Start storefront shell                                  |
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
`renderToPipeableStream`, so the document head, styles and shell reach the
browser before the page tree finishes. Buffered themes and older hosts keep the
existing JSON contract. Streaming requires the reverse proxy to pass chunked
responses through without buffering.

Themes may also declare dynamic routes in `theme.json`, for example
`"routes": { "product": "/products/{handle}", "collection": "/collections/{handle}" }`.
A published page whose `page_type` matches the route kind is used as the
template; the host resolves the referenced record and exposes it to the theme as
`data.context.route`. The sitemap only advertises those URLs when the route and
a published template exist.

Local development uses `cookie_secure = false` and explicit `cors_origins` in `config/config.example.toml`.

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
pnpm lint && pnpm typecheck && pnpm build
```

The suite covers module rules, repository behaviour, platform auth/RBAC, cart and checkout workflows, storefront HTTP flows (catalog → cart → tax → checkout → order), cancellations and restocking, webhook signing/retries, openapi schema/type parity, SQLite write serialization under concurrency, and cross-store tenant isolation.

## Roadmap

- React admin and storefront feature UIs.
- Embedded V8 runtime adapter.
- Additional payment, shipping, tax, email and search providers.
- PostgreSQL adapter for high-traffic stores.

## License

MIT
