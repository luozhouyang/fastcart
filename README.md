# Fastcart

A lightweight, self-hosted, multi-store commerce platform built with Rust and React.

Fastcart aims to deliver the developer experience of modern headless commerce platforms while keeping deployment simple: a single Rust modular monolith, one SQLite database per store, a React admin application, and a TanStack Start storefront served through an embedded JavaScript runtime.

> **Status: Phase 0 — project scaffold.**
> The workspace, configuration loading, health API, and frontend shells are in place. Commerce modules, persistence, and the SSR runtime bridge are under active development and not yet production-ready.

## Why Fastcart

- **Simple to deploy.** Single-node, single-binary first. No mandatory Redis, PostgreSQL, or message broker.
- **Multi-store by design.** Every store owns an isolated SQLite database; store context is resolved per request.
- **Rust performance.** Async Rust with Axum and Tokio, integer-based money types, and explicit module boundaries.
- **Modern storefront.** TanStack Start and React 19 with SSR and streaming, rendered inside the Rust process.
- **Extensible core.** Provider interfaces, capability-scoped plugins, and an event/outbox foundation.
- **AI-ready.** A controlled tool registry that calls application services — never storage directly — with auditable tasks.

## Architecture

```text
fastcart-server
│
├── Axum HTTP server
│
├── Commerce core
│
├── Store registry / per-store SQLite databases
│
├── Storefront runtime manager
│   ├── Embedded V8 runtime (deno_core)
│   ├── Node.js sidecar (fallback)
│   └── Worker pool + request queue
│
├── Embedded TanStack Start bundle
│
└── Static assets
```

The backend is a modular monolith. Domain crates stay independent of HTTP and SQLx so that presentation, persistence, and runtime adapters can evolve separately.

## Repository layout

| Path                        | Purpose                                                                                                        |
| --------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `apps/server`               | Axum HTTP server, bootstrap, configuration, graceful shutdown                                                  |
| `apps/admin`                | React 19 + TanStack Router admin shell                                                                         |
| `apps/storefront`           | TanStack Start storefront application                                                                          |
| `crates/kernel`             | Identifiers, money, request/store context, shared errors                                                       |
| `crates/platform`           | Store registry, domains, lifecycle, users                                                                      |
| `crates/commerce`           | Catalog, pricing, inventory, carts, checkout, orders (in progress)                                             |
| `crates/infrastructure`     | SQLite, migrations, storage, and job implementations                                                           |
| `crates/api`                | HTTP presentation layer and route namespaces                                                                   |
| `crates/storefront`         | Theme manifests, page schemas, rendering contracts                                                             |
| `crates/storefront-runtime` | Runtime API, V8 runtime, Node sidecar, pool, and bridge                                                        |
| `crates/plugin`             | Plugin manifests and declared capabilities                                                                     |
| `crates/ai`                 | Tool definitions and agent task contracts                                                                      |
| `crates/cli`                | `fastcart` administrative CLI                                                                                  |
| `packages/*`                | TypeScript SDKs and shared UI (`api-client`, `admin-sdk`, `storefront-sdk`, `theme-sdk`, `shared-types`, `ui`) |
| `themes/default`            | Default theme manifest, settings schema, and assets                                                            |
| `plugins/examples`          | Example plugin workspace                                                                                       |
| `migrations`                | Platform and per-store SQL migrations                                                                          |
| `config`                    | Example server configuration                                                                                   |
| `scripts`                   | Development, build, and verification helpers                                                                   |

## Technology stack

| Layer      | Technology                                                        |
| ---------- | ----------------------------------------------------------------- |
| Backend    | Rust 2024 edition, Axum, Tokio, SQLx, SQLite (WAL)                |
| Admin      | React 19, TanStack Router, Tailwind CSS 4, Vite                   |
| Storefront | TanStack Start, React 19, Vite                                    |
| JS runtime | deno_core + V8 (embedded), Node.js sidecar fallback               |
| Tooling    | Cargo workspace, pnpm workspace, ESLint, Prettier, GitHub Actions |

## Prerequisites

- Rust `1.97.1` (pinned by `rust-toolchain.toml`)
- Node.js `>= 22`
- pnpm `10.33.0` (pinned by `package.json`)

## Getting started

```bash
# Install JavaScript dependencies
pnpm install

# Create config/config.toml and the runtime data directories
cargo run -p fastcart-cli -- init

# Start the Rust server (defaults to 0.0.0.0:8080)
./scripts/dev-server.sh

# In separate terminals, start the frontends
pnpm dev:admin
pnpm dev:storefront
```

Verify the server:

```bash
curl http://localhost:8080/health
```

The default storefront dev server listens on `http://localhost:3000`.

### CLI

```bash
cargo run -p fastcart-cli -- init      # create directories and an initial config
cargo run -p fastcart-cli -- doctor    # validate configuration and print details
cargo run -p fastcart-cli -- serve     # start the HTTP server
```

## Configuration

Configuration is loaded from `config/config.toml`, created from `config/config.example.toml`. Environment variables can override selected values after the file is loaded:

```bash
FASTCART_SERVER_HOST=0.0.0.0
FASTCART_SERVER_PORT=8080
FASTCART_LOG_LEVEL=info
```

Key settings include the platform database path, the per-store database directory, the storage root, cache capacity, storefront runtime selection, and job worker counts. See `config/config.example.toml` for the full layout.

## Development

```bash
# Run the full verification suite (format, lint, tests, frontend builds)
./scripts/check.sh

# Or run individual commands
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm lint
pnpm build
pnpm typecheck
pnpm test
```

CI runs the Rust and web jobs on every push and pull request.

## API surface

Phase 0 exposes health endpoints and reserves the API namespaces defined by the architecture:

- `GET /health`
- `GET /health/live`
- `GET /health/ready`
- Reserved: `/api/v1/platform`, `/api/v1/admin`, `/api/v1/store`, `/api/v1/webhooks`, `/internal/storefront`

Reserved routes return `501 Not Implemented` with a structured error body until their phase lands.

## Roadmap

- [x] Workspace scaffold, configuration, health API, frontend shells
- [ ] Platform store registry and store lifecycle
- [ ] Commerce core: catalog, pricing, inventory, cart, checkout, orders
- [ ] SQLite persistence, migrations, outbox, background jobs
- [ ] Storefront runtime bridge, theming, and page rendering
- [ ] Admin product and order management
- [ ] Plugin system with capability enforcement
- [ ] AI tool registry and auditable agent tasks

## Contributing

Contributions are welcome. Please keep changes scoped to a single concern, run `./scripts/check.sh` before opening a pull request, and follow the existing crate and package boundaries.

## License

[MIT](LICENSE)
