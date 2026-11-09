---
type: ADR
title: Universal SSR runtime with workerd
description: Run framework-built storefront artifacts in a process-isolated workerd runtime instead of maintaining Node or V8 adapters.
owner: luozhouyang
status: Accepted
implementation: not-started
timestamp: 2026-11-09T02:46:06Z
---

# 0007. Universal SSR runtime with workerd

## Context

FastCart storefront rendering has been shaped around two adapters: a Node
sidecar that imports trusted theme bundles, and an embedded V8 adapter that has
never advanced past a stub. Both bind the platform to a specific rendering
stack. Themes import React, the host sends theme-specific render payloads, and
the Rust runtime API exposes `v8` and `node` as deployment-level switches.
Frameworks such as TanStack Start, Astro, SvelteKit and React Router can already
produce server bundles that export a standard Fetch handler; none of them needs
a theme-specific Node sidecar or an embedded V8 adapter.

At the same time, an embedded JavaScript engine would make FastCart own module
resolution, Web APIs, isolate limits and framework compatibility. That is a
runtime project, not a commerce platform responsibility. The current
`runtime-v8` stub and the Node sidecar protocol also keep the deployment matrix
tied to FastCart-specific adapters instead of a reusable artifact contract.

The full architecture and phased plan are recorded in
[Universal SSR Runtime](../architecture/universal-ssr-runtime.md).

## Decision

We will adopt a framework-agnostic universal SSR runtime built on
[workerd](https://github.com/cloudflare/workerd) and remove the Node/V8 adapter
line as the product rendering architecture.

1. `fastcart-serve` remains the single public HTTP entry point and owns store
   resolution, authentication, commerce APIs, static assets, the admin,
   database access and SSR lifecycle management. It does not embed V8.
2. Storefront rendering runs in a separate, process-isolated `workerd` process.
   `fastcart-serve` forwards requests over private loopback HTTP and streams the
   response body back without buffering the full page.
3. The runtime contract is a standard Worker Fetch handler:
   `export default { fetch(request, env, ctx) }`. FastCart does not know about
   React, TanStack Router, Astro, Svelte or any other framework at runtime.
4. Framework integration happens at build time. A framework build produces a
   FastCart SSR Artifact with a versioned `fastcart-ssr.json` manifest, a Worker
   entry, client assets and static files. TanStack Start is the first supported
   framework; Astro is the second.
5. A new `fastcart-ssr` Rust crate owns artifact validation and storage, the
   deployment registry and lifecycle, workerd configuration generation, process
   supervision, health checks, the SSR gateway, trusted context injection,
   streaming, timeouts and metrics.
6. Store context is resolved by `fastcart-serve` from the trusted hostname and
   is injected into the SSR request. Client-supplied internal context headers
   are stripped. The Worker cannot choose another store by changing a header or
   a path.
7. The SSR Worker calls the Commerce API over an internal HTTP namespace. It
   never receives database access, payment secrets, admin credentials or other
   backend secrets. Static assets are served by Rust, not by workerd.
8. `runtime-node`, `apps/theme-sidecar`, `runtime-v8` and the `StorefrontRuntime`
   adapter switching are legacy. They are not extended, are off the default
   deployment path, and are removed once the workerd path reaches parity on the
   supported storefront fixtures. `storefront.runtime = "node" | "v8"` is not
   part of the target configuration.
9. workerd is pinned to a tested version and compatibility date. It is managed
   as a local child process or a separately supervised process; FastCart does
   not require Docker to run it.
10. workerd is not treated as a hardened sandbox. Trusted artifacts may share a
    runtime group; untrusted third-party code requires a stronger OS or VM
    boundary.

**Rejected alternatives:**

- **Implement the embedded V8 adapter and keep Node as a fallback.** Rejected
  because it makes FastCart maintain a JavaScript runtime, a theme-specific
  payload protocol and two adapters for every framework, while still not
  accepting standard Worker artifacts.
- **Keep the Node sidecar as the only runtime.** Rejected because it couples
  every storefront to Node and React, cannot run standard TanStack Start or
  Astro Worker output unmodified, and makes process isolation a theme detail
  rather than a deployment contract.
- **Run one runtime per frontend framework.** Rejected because it multiplies
  process management, security review and health checks for no product benefit.
  Framework differences belong in build adapters.
- **Run workerd in-process.** Rejected because process isolation is a primary
  reason to use workerd; a crash or runaway artifact must not take down the
  commerce API.
- **Build a custom Rust-to-JavaScript binding or expose SQLite to the Worker.**
  Rejected because it moves commerce rules and tenant data across the security
  boundary and undermines the existing module and database ownership model.

## Consequences

**Easier / gained:**

- One runtime contract for every supported framework; adding a framework is a
  build-time adapter, not a new Rust runtime.
- A Worker crash, runaway request or memory spike is contained by the workerd
  process and can be restarted without restarting the commerce API.
- Standard Fetch, Request, Response, streams, Web Crypto and timers are provided
  by a maintained runtime rather than reimplemented in Rust.
- Deployments become versioned artifacts with validation, health checks,
  rollback and a stable manifest instead of in-place theme bundle swaps.

**Harder / accepted trade-offs:**

- A deployment now has at least two processes: `fastcart-serve` and `workerd`.
  This is not a single-binary deployment.
- FastCart must pin, ship or locate a workerd binary and manage its lifecycle,
  configuration, logs and upgrades.
- The Node sidecar and the `runtime-v8`/`runtime-node` crates remain as legacy
  code until the migration is complete; maintaining both paths during the
  transition is temporary debt.
- Framework-specific compatibility is not automatic. Cloudflare adapters may
  require bindings or platform features that FastCart does not implement, so
  each supported framework needs a real fixture and compatibility tests.
- workerd isolates are not an OS security boundary. Untrusted artifacts still
  require process, container or VM isolation and restricted filesystem and
  network access.

**Follow-ups:**

- Complete Phase 0 workerd validation before building the full Runtime Manager.
- Build the FastCart SSR Artifact schema, packager and Rust validator.
- Implement the gateway with streaming, trusted context and timeout handling.
- Add the TanStack Start fixture, then Astro, and remove the legacy adapters
  only after both pass the acceptance tests.
- Docker-based deployment documentation and artifacts have been removed; keep the
  binary + workerd process model as the only documented deployment path.

## Verification

**Implementation impact:** `crates/fastcart-ssr`, `apps/server`, `crates/api`,
`packages/storefront-sdk`, `packages/storefront-build`,
`packages/storefront-adapter-tanstack`, `packages/storefront-adapter-astro`,
`templates/storefront-tanstack`, `templates/storefront-astro`,
`schemas/storefront-artifact`, `README.md`, `.livingdocs/architecture`.

**Verification criteria:**

- A standard Fetch Handler artifact produces HTML through
  `fastcart-serve -> workerd -> browser` with the response body streamed.
- TanStack Start SSR, route loaders, server functions, cookies, redirects and
  streaming pass against the pinned workerd version.
- An Astro artifact runs on the same gateway without framework-specific Rust
  code.
- Spoofed `X-FastCart-Store-Id`, deployment and internal-token headers are
  ignored or rejected; cross-store access fails.
- An invalid artifact, path traversal, missing entrypoint and digest mismatch
  are rejected before activation.
- A worker crash, timeout and unavailable runtime produce controlled responses
  and do not crash `fastcart-serve`.
- The production path does not start `apps/theme-sidecar`, `runtime-node` or
  `runtime-v8`.
- `cargo test --workspace`, clippy and the frontend test/build suites pass.

## Implementation status

Not started. The current repository still contains the legacy Node sidecar and
the `runtime-v8` stub. This ADR records the target architecture; it does not
claim that workerd integration exists. The legacy adapters must not be extended
while the workerd migration is in progress.
