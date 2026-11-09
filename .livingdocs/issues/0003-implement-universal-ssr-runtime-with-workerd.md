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
