---
type: Issue
title: Implement embedded V8 SSR with runtime switching
description: Make runtime-v8 a real deno_core adapter with v8/node/auto selection and parity tests.
status: superseded
labels: [area:ssr]
timestamp: 2026-11-06T16:18:26Z
superseded_by: 0003
---

## 0001. Implement embedded V8 SSR with runtime switching

> **Superseded by [Issue 0003](0003-implement-universal-ssr-runtime-with-workerd.md)
> and [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md).**
> Do not implement `runtime-v8`, `runtime-node` switching or the `auto` fallback.
> The universal workerd runtime replaces both adapters; the scope and
> acceptance criteria below are historical and must not be executed.

Implements [ADR 0002](../adr/0002-embedded-v8-ssr-with-a-switchable-node-fallback.md) and the storefront requirement in [PRD 0001](../prd/0001-fastcart-v1-platform-requirements.md). The Node sidecar is the only working SSR adapter today; `runtime-v8` is a stub. This work makes V8 a real adapter and lets a deployment choose `v8`, `node` or `auto` without rebuilding.

### Scope

- `runtime-v8` backed by `deno_core`: ESM module loading from the registered bundle, the Web APIs the theme contract needs (streams, encoding, timers, URL, crypto, fetch), per-request timeout, stream deadline, heap limit and ordered chunk delivery.
- A self-contained theme server bundle: React and runtime dependencies are bundled into the server entry; Node built-ins are either absent or rejected with a clear error.
- Runtime selection in `apps/server`: `storefront.runtime = "v8" | "node" | "auto"`, with `auto` preferring V8 and falling back to Node only when a sidecar is configured and V8 initialization fails.
- Theme compatibility: activation continues to compare the manifest's declared runtime with the selected adapter.
- Health, structured errors and documentation for both adapters.

### Decision

The theme build will own the self-contained-server-bundle rule rather than the V8 loader resolving `node_modules`. This keeps the runtime contract identical to the Node sidecar's JSON payload and avoids a Node-specific resolver in Rust.

### Acceptance

- The default theme produces equivalent HTML on V8 and Node for a fixed request fixture.
- V8 streaming yields ordered chunks and enforces the request and stream deadlines.
- A theme declaring `runtime: "node"` cannot activate while the deployment selects V8.
- Changing `FASTCART_STOREFRONT_RUNTIME` between `v8` and `node` starts the same store and theme without a rebuild.
- `cargo test --workspace` and the sidecar protocol tests pass; V8 has its own render and timeout tests.

### Plan

1. Build a self-contained `themes/default` server bundle and prove it runs in Node without `node_modules` resolution.
2. Add `deno_core` and a minimal V8 runtime that imports the bundle and calls `render`, returning a buffered body.
3. Add Web APIs, streaming and the request/stream deadlines.
4. Add runtime selection and `auto` fallback in server bootstrap, with health reporting.
5. Add parity tests against the Node adapter and document the supported API surface.
6. Wire the admin/storefront runtime compatibility checks and update deployment docs.
