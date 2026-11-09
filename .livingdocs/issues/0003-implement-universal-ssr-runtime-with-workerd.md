---
type: Issue
title: Implement the universal SSR runtime with workerd
description: Replace the Node/V8 adapters with the workerd runtime, artifact pipeline and SSR gateway.
status: open
labels: [area:ssr]
timestamp: 2026-11-09T02:46:06Z
---

## 0003. Implement the universal SSR runtime with workerd

Implements [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) and the
phased plan in [Universal SSR Runtime](../architecture/universal-ssr-runtime.md).
The Node sidecar and the `runtime-v8` stub are legacy and must not be extended.
The target architecture runs framework-built Worker artifacts in a
process-isolated `workerd` process managed by `fastcart-serve`.

### Scope

- Phase 0: pin a workerd binary and validate Fetch handlers, Web APIs, streaming,
  service bindings, TanStack Start output and Astro output.
- `crates/fastcart-ssr`: artifact manifest parsing and validation, immutable
  artifact storage, deployment registry and lifecycle, workerd configuration
  generation, process supervisor, health checks, gateway and trusted context.
- A versioned FastCart SSR Artifact (`fastcart-ssr.json`) and a TypeScript
  packager under `packages/storefront-build`.
- TanStack Start as the first supported framework, then Astro, using official
  Cloudflare build integrations and FastCart fixtures.
- Streaming request/response proxying, timeouts, error mapping, request IDs,
  metrics and structured logs.
- Static asset serving through Rust at `/_fastcart/assets/`.
- Internal Commerce API access for SSR Workers with a scoped credential; no
  database or backend secrets in the Worker.
- Documentation and deployment updates; removal of Docker as a supported path.

### Acceptance

- A standard Fetch Handler artifact renders through
  `fastcart-serve -> workerd -> browser` with streamed HTML.
- TanStack Start SSR, loaders, server functions, cookies, redirects and
  streaming pass against the pinned workerd version.
- Astro runs on the same gateway without framework-specific Rust code.
- Spoofed store/deployment/internal-token headers cannot cross tenant
  boundaries; artifact path traversal and digest mismatches are rejected.
- Worker crashes and timeouts return controlled errors without crashing
  `fastcart-serve`.
- The production path does not start `apps/theme-sidecar`, `runtime-node` or
  `runtime-v8`; those crates are removed after parity.
- `cargo test --workspace`, clippy and the frontend test/build suites pass.

### Plan

Follow the phases in the architecture plan. Do not start Phase 1 until the Phase
0 acceptance criteria pass. Keep the legacy adapters working only until the
workerd fixtures substitute for them; do not add features to the legacy path.

### Phase 0 status

Validated on 2026-11-09 with workerd `v1.20261109.1`; see
[ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md#implementation-status)
and `experiments/workerd`. The basic Fetch handler, Web APIs, streaming, service
bindings, TanStack Start and Astro fixtures pass.

### Phase 1 status

`crates/fastcart-ssr` now starts a supervised workerd process, proxies
requests with streamed responses, reports health and shuts down with SIGTERM.
The gateway is not mounted in `fastcart-server` yet; Phase 4 owns that wiring.

### Phase 2 status

The artifact manifest schema, TypeScript packager and Rust validator/store are
in place. Rust and TypeScript share a digest fixture, and traversal, symlinks,
missing entries and size limits are enforced.

### Phase 3 status

`RuntimeManager` now generates workerd configuration for a stored artifact and
manages group start, health, stop and restart with backoff and a restart limit.
Background crash monitoring and persisted group state remain Phase 8/9 work.

### Phase 4 status

The storefront fallback forwards to a managed workerd group. Trusted context is
injected, spoofed internal headers are removed, response streams and duplicate
`Set-Cookie` headers are preserved. `[ssr]` configuration is wired into
`apps/server`.

### Phase 5 status

`templates/storefront-tanstack` builds a FastCart SSR artifact and its workerd
smoke test covers SSR, server functions, trusted headers, streaming, cookies,
redirects and errors. A manual `fastcart-server` run rendered the template.
Static assets and browser hydration are Phase 7 work.

### Phase 6 status

The server-only `/internal/ssr/v1/stores/{store_id}` namespace requires a bearer
token and resolves the active store from the path. `@fastcart/storefront-sdk/server`
rewrites public SDK paths to this namespace. Token scoping and Worker-side SDK
use remain Phase 8 work.

### Phase 7 status

Rust serves active-artifact client assets at `/_fastcart/assets/*` with
traversal protection, content types and immutable caching. The TanStack template
uses that base without changing its router base. Per-store artifacts, range
requests and compression remain later phases.

### Phase 8 status

`[[ssr.deployments]]` selects an isolated workerd group per store and assets are
served from the matching artifact. Unknown stores return 404. The registry is
boot-time configuration; activation, rollback, draining and per-store tokens
remain Phase 9.

### Phase 9 status

Authenticated activation and rollback start a replacement, wait for health,
swap atomically and drain the old group. Retired asset roots stay readable
during the drain window. Activation is not restart-persistent and artifact paths
are not root-confined yet.

### Phase 10 status

Activation paths are confined by `ssr.artifact_root`; successful default-mode
activation is written atomically and reloaded on restart. Render/activation/drain
paths emit structured tracing. Per-store credentials and metrics remain open.
