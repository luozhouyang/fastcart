# Fastcart

A lightweight, self-hosted, multi-store commerce platform built with Rust and React.

Fastcart aims to deliver the developer experience of modern headless commerce platforms while keeping deployment simple: a single Rust modular monolith, one SQLite database per store, a React admin application, and an embedded storefront runtime.

> **Status: backend implementation complete; UI and SSR runtime are still being built.**
> The platform, commerce modules, workflows and HTTP APIs are implemented and
> covered by unit and integration tests. The React admin/storefront applications
> are shells. Storefront runtime adapters (worker pool, in-process fetch bridge
> and Node sidecar) exist, but are not yet wired into the server.

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

### Cross-cutting

- Transactional outbox with retry/backoff and in-process event bus.
- SQLite-backed persistent job queue with atomic claiming and lease expiry.
- Signed webhook delivery from outbox events with idempotent fan-out, retry/backoff, crash recovery and replay.
- Periodic maintenance: cart, reservation and checkout expiry; outbox dispatch; webhook delivery; simple job handlers.
- Store-scoped Moka cache with generation-based invalidation.
- Prometheus-compatible `/metrics` with latency histograms, plus baseline security response headers.
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

The suite covers module rules, repository behaviour, platform auth/RBAC, cart and checkout workflows, storefront HTTP flows (catalog → cart → tax → checkout → order), cancellations and restocking, webhook signing/retries and cross-store tenant isolation.

## Roadmap

- React admin and storefront feature UIs.
- Wire the storefront runtime into the server and implement theme page rendering.
- Embedded V8 runtime adapter.
- OpenAPI generation and generated TypeScript SDKs.
- Additional payment/shipping/tax providers.
- PostgreSQL adapter for high-traffic stores.

## License

MIT
