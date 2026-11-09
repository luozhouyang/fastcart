---
type: ADR
title: Universal SSR runtime with workerd
description: Run framework-built storefront artifacts in a process-isolated workerd runtime instead of maintaining Node or V8 adapters.
owner: luozhouyang
status: Accepted
implementation: partial
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

Phase 0 is validated; the production runtime is not implemented.

Phase 0 (2026-11-09) used workerd `v1.20261109.1` (Linux x86_64, SHA-256
`543f30be7dc9b6e263226fa08daa86cbb505b4b407048fbcdb82d0f1b910ad80`) and passed:

- A standard ES-module Fetch handler served a request.
- Request, Response, Headers, URL, ReadableStream/WritableStream, encoding, Web
  Crypto, AbortController, timers and incremental streaming worked.
- A Worker called another workerd service through a service binding.
- A TanStack Start `1.169.5` Cloudflare build and an Astro `7.4.7` +
  `@astrojs/cloudflare` `14.4.6` build both served SSR documents.

Phase 0 also confirmed two constraints for the artifact design: framework output
may be a module graph that needs bundling or multi-module loading, and
Cloudflare-specific bindings such as `ASSETS`, `IMAGES` and `SESSION` must be
declared or rejected rather than assumed. The fixtures and commands are in
`experiments/workerd` (gitignored binary; see `scripts/install-workerd.sh`).

Phase 31 (2026-11-09) added a bounded positive cache for preview asset
revocation checks with same-process admin invalidation. Cross-instance
invalidation and cache metrics remain open.

Phase 30 (2026-11-09) made preview asset authorization honor the token's
revocation record, so revoking a preview `jti` blocks assets immediately.
Cross-origin preview hosting and authorization caching remain open.

Phase 29 (2026-11-09) required a store-bound, HttpOnly signed preview cookie
for `/_fastcart/preview-assets/*`, added `Vary: Cookie`, and registered the exact
`/preview/` route. Asset-time JTI revocation remains open.

Phase 28 (2026-11-09) added preview-specific asset bases for TanStack Start,
Astro and SvelteKit, including preview build/smoke scripts and CI coverage.
Separate builds are required; preview asset authorization remains open.

Phase 27 (2026-11-09) added a startup scan that removes artifact digest
directories not referenced by the deployments started in that process.
Concurrent runtime writers and sweep metrics remain open.

Phase 26 (2026-11-09) added reference-counted artifact garbage collection after
drain, retaining shared digest directories and pruning asset roots. Startup
orphan sweeping and GC metrics remain open.

Phase 25 (2026-11-09) added credential backend metrics and opt-in
XChaCha20-Poly1305 file encryption with an environment-provided key. Key
rotation, KMS integration and automatic plaintext migration remain open.

Phase 24 (2026-11-09) extracted credential storage into
`fastcart-ssr-credentials`, added a `[ssr.credentials]` backend/mode/refresh
configuration, persist-before-swap rotation, read-only mode, polling refresh and
file CAS. Kubernetes/Vault backends remain out of scope; backend metrics and
encryption remain open.

Phase 23 (2026-11-09) added platform audit records for activation, rollback,
preview and credential lifecycle actions, excluding token values. Service
identity, IP attribution and tamper-evident storage remain open.

Phase 22 (2026-11-09) added explicit scoped-token revocation through the
internal lifecycle API. Audit events, token fingerprints and multi-instance
coordination remain open.

Phase 21 (2026-11-09) isolated preview assets under
`/_fastcart/preview-assets/*`, propagated a trusted asset-prefix header, and
made the generic template use it. Framework preview builds and asset GC remain
open.

Phase 20 (2026-11-09) isolated preview credentials: preview renders select only
credentials marked preview, those credentials are read-only on store routes,
and preview deployments without one receive no internal token. Field-level draft
policies and preview asset roots remain open.

Phase 19 (2026-11-09) added a persistent, atomically written credential store
with Unix `0600` mode, rotation reload and expired-token filtering. Secret
manager integration, revocation and multi-instance coordination remain open.

Phase 18 (2026-11-09) added online token rotation with a grace window, mutable
shared credentials, per-request gateway token selection and expiry purging.
Rotation is in memory and reverts to config on restart.

Phase 17 (2026-11-09) added per-deployment internal credentials with store
scoping and SSR Prometheus metrics. Token rotation and preview data isolation
remain productization work.

Phase 16 (2026-11-09) added a SvelteKit template. The adapter Worker is bundled
with wrangler dry-run, `_app` assets are served through a compatibility route,
and generated workerd configs include a no-op cache service for Cache API
callers. Real shared cache and browser hydration remain future work.

Phase 15 (2026-11-09) added a framework-free Fetch Handler template and smoke
test. The core gateway required no changes, validating the artifact contract.
Additional framework adapters remain future work.

Phase 14 (2026-11-09) added opt-in route-level SSR caching controlled by the
Worker's `Cache-Control`, with store-wide invalidation on mutations, preview
bypass and cache HIT/MISS headers. SWR and shared/persistent caches remain
productization work.

Phase 13 (2026-11-09) added separate preview deployments. Preview entries are
activated through the internal token, selected only by `/preview` after signed
token verification, and inject `x-fastcart-preview` while production remains
untouched. Preview-specific assets and draft data injection remain follow-up
work.

Phase 12 (2026-11-09) added ETag/`If-None-Match`, single byte ranges,
`Accept-Ranges` and a conservative `no-store` default for SSR documents. Asset
compression, universal preview and asset retention remain productization work.

Phase 11 (2026-11-09) added `templates/storefront-astro` and an `ASSETS` disk
binding in generated workerd configs. Astro SSR, trusted headers, streaming,
cookies, redirects and errors pass the workerd smoke test and a manual
`fastcart-server` render; framework-specific bindings remain future work.

Phase 10 (2026-11-09) added artifact-root confinement, atomic activation-state
persistence for default mode, persisted-state reload and structured render
tracing. Per-store credentials and SSR metrics remain hardening work.

Phase 9 (2026-11-09) added authenticated activation and rollback. A replacement
starts and becomes healthy before the active pointer swaps; the old group drains
on a timer and its asset roots remain readable. Activation is in memory and
reverts to config on restart; persistence and artifact-root confinement remain
later work.

Phase 8 (2026-11-09) added a config-backed multi-store registry. Each
`[[ssr.deployments]]` entry gets an isolated workerd group; storefront requests
select by resolved store id and unknown stores return 404. Asset serving resolves
the store from `Host`. Database-backed activation, rollback and per-store
credentials remain Phase 9.

Phase 7 (2026-11-09) added Rust static-asset serving at `/_fastcart/assets/*`
from the active artifact, with traversal rejection, content types and immutable
caching. The TanStack template now emits asset URLs under that prefix while
keeping its router base at `/`. Per-store artifact selection and range/compressed
responses remain later phases.

Phase 6 (2026-11-09) added the server-only commerce namespace
`/internal/ssr/v1/stores/{store_id}` and
`@fastcart/storefront-sdk/server`. The namespace requires a bearer token,
resolves the active store from the path and reuses the public store handlers.
Per-deployment token scoping and SDK execution inside the Worker remain Phase 8
work.

Phase 5 (2026-11-09) added `templates/storefront-tanstack`, a TanStack Start
template that builds a real Worker artifact and packages it with
`@fastcart/storefront-build`. A workerd smoke test covers SSR, server functions,
trusted headers, streaming, duplicate cookies, redirects and errors; a manual
`fastcart-server` run confirmed the full storefront path. Static asset serving
and browser hydration remain Phase 7 work.

Phase 4 (2026-11-09) wired the runtime into `fastcart-server`. The storefront
fallback resolves the store, then forwards the request to a managed workerd
group; the gateway strips client-supplied internal headers and injects trusted
store/deployment/request context, preserves multiple `Set-Cookie` headers and
streams the response body. `[ssr]`/`FASTCART_SSR_*` configuration imports and
starts the artifact. Browser-level framework coverage arrives in Phase 5; static
assets, multi-store selection and crash-triggered restarts remain later phases.

Phase 3 (2026-11-09) added `RuntimeManager` and a deterministic Cap'n Proto
generator. A manager starts, health-checks, stops and restarts a runtime group,
applies a restart backoff and stops after the configured restart limit. Configs
are generated under `groups/<id>/config.capnp` with the entry module first.
Background crash monitoring, persisted group state, log files and bindings are
still later-phase work.

Phase 2 (2026-11-09) added the versioned `fastcart-ssr.json` schema, a
TypeScript packager (`packages/storefront-build`) and the Rust artifact
parser/validator/content-addressed store. Rust and TypeScript share a
cross-language digest fixture; missing entries, traversal, symlinks, duplicate
capabilities and oversized trees are rejected. Stored artifacts are immutable by
digest addressing; OS read-only permissions wait for deployment activation.

Phase 1 (2026-11-09) added `crates/fastcart-ssr` with a supervised
`WorkerdProcess` (SIGTERM, forced-kill deadline, log capture, readiness probe)
and an `SsrGateway` that forwards HTTP requests and preserves the response body
as a stream. The gateway is not yet mounted in `fastcart-server`; that is Phase 4.
Request bodies are buffered `Bytes` in Phase 1, and the caller still supplies
the workerd Cap'n Proto file. The integration test starts a real workerd child,
proxies POST/GET requests, checks status and headers, and verifies shutdown; it
skips when no pinned binary is available.

The current repository still contains the legacy Node sidecar and the
`runtime-v8` stub. These are transitional and must not be extended while the
workerd migration is in progress.
