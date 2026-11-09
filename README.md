# Fastcart

A lightweight, self-hosted, multi-store commerce platform built with Rust and React.

Fastcart aims to deliver the developer experience of modern headless commerce platforms while keeping deployment simple: a single Rust modular monolith, one SQLite database per store, a React admin application, and a process-isolated universal SSR runtime.

> **Status: broad Rust backend, an operational Admin, and a storefront runtime in migration.**
> The Rust platform, commerce modules, workflows, HTTP APIs, theme registry,
> webhook and email delivery, observability and OpenAPI document are implemented
> and covered by unit and integration tests. Typed TypeScript SDKs provide the
> front-end client surface. The React Admin covers orders, catalog, inventory,
> customers, promotions, content, themes, forms, inbox, notifications, regions,
> shipping, tax and settings. `apps/storefront` is a TanStack Start reference
> harness. The universal `workerd` SSR path is implemented
> ([ADR 0007](.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md));
> TanStack Start/Astro templates and the default artifact build are still in
> progress. The legacy Node sidecar remains transitional until parity.

## Why Fastcart

- **Simple to deploy.** Single-node first. The commerce server is one binary; SSR runs in a supervised, process-isolated runtime. No mandatory Docker, Redis, PostgreSQL or message broker.
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
- Storefront SSR: the accepted target is a framework-agnostic universal runtime. `fastcart-serve` resolves the store, strips client-supplied internal headers, injects trusted context and forwards the request to a process-isolated `workerd` runtime, which executes a standard Fetch-handler artifact and calls the internal Commerce API. The legacy Node sidecar (worker thread, heap and wall-clock limits, no CPU isolation) remains transitional and must not be extended; see [ADR 0007](.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md).
- Periodic maintenance: cart, reservation and checkout expiry; outbox dispatch; webhook delivery; email jobs; simple job handlers.
- Store-scoped Moka cache with generation-based invalidation.
- Prometheus-compatible `/metrics` with latency histograms, plus baseline security response headers.
- OpenAPI 3.1 schemas generated from the Rust response types with `utoipa`, with a cached document and schema-reference validation.
- Local object storage with path-traversal protection.
- Plugin manifest registry with capability checks and dependency resolution.
- AI provider/model/tool registries with an auditable agent-task state machine.

## Repository layout

| Path                            | Purpose                                                           |
| ------------------------------- | ----------------------------------------------------------------- |
| `apps/server`                   | Axum server bootstrap, configuration, graceful shutdown           |
| `apps/admin`                    | React Admin application for the implemented store operations      |
| `apps/storefront`               | TanStack Start reference harness (first universal-runtime target) |
| `templates/storefront-tanstack` | TanStack Start starter and artifact packager                      |
| `crates/kernel`                 | IDs, money, contexts, validation, errors                          |
| `crates/platform`               | Platform DB, stores, domains, auth, RBAC, provisioning            |
| `crates/infrastructure`         | SQLite pools, migrations, cache, outbox, jobs, storage            |
| `crates/http`                   | Error envelope, extractors, auth/store middleware, rate limiting  |
| `crates/modules/*`              | The independent commerce modules listed above                     |
| `crates/workflows`              | Cart pricing, checkout and maintenance workflows                  |
| `crates/api`                    | Versioned HTTP surface and DTOs                                   |
| `crates/plugin`, `crates/ai`    | Extension foundations                                             |
| `migrations/platform`           | Platform schema                                                   |
| `migrations/store`              | Per-store schema (catalog, pricing, inventory, orders, …)         |

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
- `FASTCART_STOREFRONT_ENABLED` — enable SSR (default off)
- `FASTCART_STOREFRONT_STREAMING`, `FASTCART_STOREFRONT_STREAM_TIMEOUT_MS` — allow streamed HTML (default on) and its total time budget (default 30s)
- `FASTCART_THEMES_DIR` — trusted theme bundle directory (default `./themes`); transitional until artifacts replace theme bundles
- `FASTCART_INTERNAL_FETCH_SECRET` — legacy secret-gated `/internal/storefront` surface; the target SSR gateway uses a scoped internal credential
- `FASTCART_CAPABILITY_SECRET` — signs theme capability grants so plugin endpoints can verify them
- `FASTCART_SSR_ENABLED` — enable the universal workerd runtime (requires an artifact)
- `FASTCART_SSR_BINARY`, `FASTCART_SSR_ARTIFACT`, `FASTCART_SSR_RUNTIME_DIR` — pinned workerd binary, source artifact directory and runtime data directory
- `FASTCART_SSR_LISTEN`, `FASTCART_SSR_DEPLOYMENT_ID`, `FASTCART_SSR_INTERNAL_TOKEN` — loopback listener, deployment identity and optional internal token

### Storefront SSR (target architecture)

The accepted design is a framework-agnostic universal SSR runtime:
`fastcart-serve` forwards storefront requests to a process-isolated `workerd`
runtime, which executes a standard Fetch-handler artifact and calls the internal
Commerce API. TanStack Start is the first supported framework; Astro is second.
The artifact contract, runtime manager, gateway, trusted request context and
deployment lifecycle are specified in [ADR 0007](.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md)
and the [universal SSR runtime plan](.livingdocs/architecture/universal-ssr-runtime.md).
Implementation has not started.

The legacy Node renderer (`runtime-node`, `apps/theme-sidecar`,
`FASTCART_STOREFRONT_RUNTIME=node|v8`) remains in the tree only as transitional
code. It is not the target deployment, must not be extended, and will be removed
once the workerd path passes parity. Node and Docker are not required by the
target architecture. For local migration testing only, the legacy sidecar still
starts with:

```bash
pnpm --filter @fastcart/theme-default build
SIDECAR_SECRET=change-me pnpm --filter @fastcart/theme-sidecar start
# and set FASTCART_STOREFRONT_NODE_SECRET=change-me on the server
```

### Storefront trust model

Storefront artifacts are **trusted server-side code**. Registering and activating
an artifact is a privileged, audited operation. The target runtime executes
artifacts in a process-isolated `workerd` group; a workerd isolate is a
reliability boundary, not an operating-system sandbox. Untrusted third-party code
requires a dedicated process group plus an OS or VM boundary, and storefront
code never receives database access, payment secrets, admin credentials or other
backend secrets. Static assets are served by Rust, not by the Worker. See
[ADR 0003](.livingdocs/adr/0003-trusted-theme-bundles-with-worker-limits.md).

Local development uses `cookie_secure = false` and explicit `cors_origins` in `config/config.example.toml`.

## Deployment

Docker is **not** a supported deployment path. The Dockerfiles, Compose stack,
Admin nginx image and theme-sidecar image have been removed from the repository;
do not reintroduce them for the workerd migration.

The target topology is two local processes on one host:

| Process          | Role                                                                                                         |
| ---------------- | ------------------------------------------------------------------------------------------------------------ |
| `fastcart-serve` | Public HTTP entry point, commerce API, Admin, static assets, SQLite, store resolution and SSR management     |
| `workerd`        | Pinned, process-isolated SSR runtime; executes framework-built Fetch-handler artifacts on a private listener |

`workerd` is pinned to a tested version and compatibility date. `fastcart-serve`
generates its configuration, starts and health-checks it, and restarts it with
backoff. The SSR gateway strips client-supplied internal context headers and
injects a trusted store/deployment/request context before forwarding. Static
assets are served by Rust at `/_fastcart/assets/`.

The universal runtime path is implemented and can be enabled with an artifact;
TanStack Start/Astro starter templates and the default artifact build remain
Phase 5/11 work. The legacy Node sidecar is a migration aid only; it is not the
supported deployment and must not be extended. See
[ADR 0007](.livingdocs/adr/0007-universal-ssr-runtime-with-workerd.md).

To run the universal path, keep `storefront.enabled = false` and set:

```toml
[ssr]
enabled = true
binary = "/opt/fastcart/workerd"
artifact_path = "./storefront-artifact"
runtime_dir = "./data/ssr"
listen = "127.0.0.1:18888"
deployment_id = "default"
# internal_token = "..."
```

The artifact must contain `fastcart-ssr.json` and expose
`/__fastcart/health`; the binary is the pinned workerd build from
`scripts/install-workerd.sh`. `ssr.listen` is loopback-only in this phase.
Client assets are served by Rust at `/_fastcart/assets/*`; the TanStack template
sets its Vite base to that prefix while keeping page routes at `/`.

Configuration is TOML plus environment overrides. Required and recommended
settings:

- `FASTCART_PREVIEW_SECRET` — at least 32 bytes; enables draft preview. Without
  it the preview endpoints return `501 Not Implemented`.
- `FASTCART_CAPABILITY_SECRET` — signs capability grants. Set a long random
  value in production when plugins use capability endpoints.
- `FASTCART_INTERNAL_FETCH_SECRET` — legacy shared secret for the
  `/internal/storefront` surface. The target gateway replaces it with a scoped
  internal runtime credential.
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
  cart/checkout/inquiry limits.
- `FASTCART_PREVIEW_FRAME_ANCESTORS` — the Admin origin allowed to embed the
  preview iframe, for example `https://admin.example.com`.
- `FASTCART_ADMIN_BASE_URL` — absolute Admin URL used in notification deep
  links.

TLS termination is external: put a reverse proxy in front of the loopback
`fastcart-serve` port and forward `X-Forwarded-Proto` from that proxy. The target
topology serves the Admin from `fastcart-serve`; the legacy separate Admin image
is not supported and its artifacts have been removed.

Health endpoints: `fastcart-serve` exposes `/health/live` and `/health/ready`.
The target `workerd` runtime exposes an internal `/health` endpoint that is
never public.

## HTTP API

All endpoints are under `/api/v1` unless noted.

| Namespace                  | Purpose                                                                      |
| -------------------------- | ---------------------------------------------------------------------------- |
| `/platform/*`              | Installation, auth, stores, domains, memberships                             |
| `/admin/*`                 | Store-scoped administration                                                  |
| `/store/*`                 | Public storefront and customer operations                                    |
| `/webhooks/payments/*`     | Provider callbacks                                                           |
| `/internal/*`              | SSR-to-commerce boundary (`/internal/ssr/v1`; legacy `/internal/storefront`) |
| `/health`, `/health/ready` | Liveness and readiness                                                       |
| `/metrics`                 | Prometheus-compatible process metrics                                        |
| `/api/v1/openapi.json`     | OpenAPI 3.1 document                                                         |
| `/*`                       | Storefront SSR fallback (legacy theme runtime; target gateway)               |

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

The suite covers module rules, repository behaviour, platform auth/RBAC, cart and checkout workflows, storefront HTTP flows (catalog → cart → tax → checkout → order), cancellations and restocking, webhook signing/retries, SQLite write serialization under concurrency, and cross-store tenant isolation.

`pnpm e2e:browser` is a real-browser smoke test for the `/cart` and `/checkout` deep links. It drives headless Chrome against a running server (`FASTCART_BASE_URL`, `FASTCART_HOST`), waits for hydration, and asserts the cart drawer opened. Playwright is intentionally not a dependency; the script uses the Chrome binary already required by the deployment environment. The OpenAPI tests assert a 3.1.0 document, namespace coverage and resolvable schema references; there is no TypeScript code-generation or type-parity check in the repository.

## Documentation

The living project docs — constitution, v1 PRD, decision records and architecture views — are in
[`.livingdocs/`](.livingdocs/index.md). The design notes under `docs/` are local-only working
material and are intentionally not versioned.

## Roadmap

- React admin and storefront feature UIs.
- Universal `workerd` SSR runtime, artifact pipeline and TanStack Start/Astro adapters.
- Additional payment, shipping, tax, email and search providers.
- PostgreSQL adapter for high-traffic stores.

## License

MIT
