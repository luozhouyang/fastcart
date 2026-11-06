---
type: ADR
title: Embedded V8 SSR with a switchable Node fallback
description: Implement deno_core V8 rendering and keep Node as a config-selectable fallback.
owner: luozhouyang
status: Accepted
timestamp: 2026-11-06T16:17:43Z
---

# 0002. Embedded V8 SSR with a switchable Node fallback

## Context

The SSR design chose an embedded `deno_core`/V8 runtime so a single Rust process could render themes without Node, with a Node sidecar as the documented fallback. The runtime API already separates `StorefrontRuntime` from Axum, and `runtime-v8` exists only as a stub that returns `NotInitialized`; production rendering currently requires the Node sidecar. The project owner wants the embedded runtime implemented while keeping a fast, config-only switch between runtimes.

A decision is required because maintaining two adapters changes the build, the theme contract and the deployment matrix. The current theme bundle imports `react-dom/server.browser` and `react` as bare specifiers, which V8 cannot resolve from `node_modules`; the theme build and runtime contract therefore have to be explicit.

## Decision

We will implement `runtime-v8` on `deno_core` as a first-class `StorefrontRuntime` and keep `runtime-node` as a supported fallback. Both adapters ship in the same server binary. The deployment selects one with `storefront.runtime = "v8" | "node" | "auto"`; `auto` initializes V8 and falls back to the Node sidecar only when V8 initialization fails and `node_url` is configured. A theme manifest's declared runtime must match the selected adapter, so activation cannot place a Node-only theme on V8. Switching runtimes is a configuration change plus process restart, with no rebuild and no database migration.

The V8 adapter will provide the Web APIs the theme contract requires (module loading, streams, encoding, timers, URL, crypto, fetch), enforce the request and stream deadlines plus a heap limit, and stream HTML through the existing `RuntimeBody::Stream` bridge. Theme server bundles must be self-contained: React and every runtime dependency are bundled into the server entry, and Node built-ins are either absent or explicitly declared unsupported. The Node sidecar remains the escape hatch for themes that genuinely depend on Node modules.

**Rejected alternatives:**

- **V8 only, removing the Node sidecar.** Rejected because existing Node-dependent themes would become undeployable and the sidecar is still the fastest compatibility path.
- **Node only, leaving `runtime-v8` a stub.** Rejected by the project owner; it forfeits the single-binary deployment goal.
- **Per-store or per-request hot switching.** Rejected for v1: it multiplies cache keys, health checks and theme compatibility rules for little operational value; a deployment-level switch is enough.
- **A custom JavaScript sandbox instead of `deno_core`.** Rejected because building Web APIs, modules and streaming on another engine duplicates a mature runtime.

## Consequences

**Easier / gained:**

- A deployment can run SSR with no Node process and still fall back to Node without rebuilding.
- The `StorefrontRuntime` contract stays the only interface Axum depends on; runtime selection is configuration.
- Theme bundles become deterministic, self-contained server artifacts.

**Harder / accepted trade-offs:**

- V8 adds a large native dependency, build time and binary size.
- V8 will not support arbitrary Node built-ins; themes using them must stay on the Node runtime.
- Two adapters must pass the same streaming, timeout and isolation tests; a bug fix may touch both.
- `auto` fallback can hide a broken V8 build, so initialization failures must be logged and exposed in health.

**Follow-ups:**

- Build the V8 adapter and the theme server bundle changes as one vertical slice.
- Keep the Node sidecar protocol tests as the behavioral oracle for both adapters.
- Implementation is tracked in [Issue 0001](/issues/0001-implement-embedded-v8-ssr-with-runtime-switching.md).

## Verification

**Implementation impact:** `crates/storefront-runtime/runtime-v8`, `crates/storefront-runtime/runtime-node`, `crates/api/src/state.rs`, `apps/server/src/config.rs`, `apps/server/src/state.rs`, `themes/default/vite.config.ts`, `Cargo.toml`.

**Verification criteria:**

- The default theme renders the same HTML on V8 and Node for a fixed request fixture.
- Streaming responses produce ordered chunks and enforce the configured deadlines on both adapters.
- A theme declaring `runtime: "node"` cannot activate while `storefront.runtime = "v8"`.
- Changing `FASTCART_STOREFRONT_RUNTIME` between `v8` and `node` starts the server with the same database and theme without a rebuild.
- `cargo test --workspace` and the sidecar protocol tests pass.
