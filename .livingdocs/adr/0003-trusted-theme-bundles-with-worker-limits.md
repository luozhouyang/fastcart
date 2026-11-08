---
type: ADR
title: Trusted theme bundles with worker limits
description: Treat theme bundles as trusted server-side code isolated by worker resource limits, not an OS sandbox.
owner: luozhouyang
status: Accepted
implementation: partial
timestamp: 2026-11-06T16:17:43Z
---

# 0003. Trusted theme bundles with worker limits

## Context

Theme bundles render server-side and therefore execute code with access to the host. The platform explicitly excludes arbitrary untrusted server-side code from its first release. The current Node sidecar runs each render in a worker thread with heap caps and a hard deadline, but the worker can still import filesystem and network APIs. Registration and activation are admin-only operations, so a theme is closer to an installed server plugin than to user content.

A decision is required because calling the worker a "sandbox" would set a security expectation the implementation does not meet, while refusing to run themes at all would remove the product's rendering model.

## Decision

We will treat theme bundles as trusted server-side code. Theme execution runs in worker threads with heap limits, a hard render deadline, a bounded response size and asset paths confined to the registered bundle; the Node sidecar binds to loopback and uses a bearer secret. We will not claim OS-level isolation for themes. Deployments that accept untrusted themes must use a separate sidecar per trust domain, a container with a read-only filesystem and restricted egress, or wait for an isolate-level runtime. Registering and activating a theme is a privileged admin operation and must be audited.

**Rejected alternatives:**

- **Node `vm` contexts or global patching as a sandbox.** Rejected because `vm` is not a security boundary and ESM imports bypass patched globals.
- **A child process or container per render.** Rejected for v1 because the latency and lifecycle cost is disproportionate for trusted bundles; the V8 runtime is the intended stronger boundary for a future untrusted mode.
- **WASM-only themes.** Rejected because the React/SSR ecosystem and the existing theme bundles cannot be expressed that way without a rewrite.

## Consequences

**Easier / gained:**

- A runaway theme cannot block the event loop or allocate without bound; the health endpoint stays responsive.
- The theme contract remains the ordinary Node/React toolchain.
- Deployment guidance is honest about the trust boundary.

**Harder / accepted trade-offs:**

- A malicious registered theme can read files and use the network; the operator must protect the sidecar host.
- Untrusted multi-tenant theme hosting remains out of scope until V8 or an external sandbox provides the boundary.

**Follow-ups:**

- Keep the trust model in the README and the sidecar deployment guide.
- Revisit the boundary when `runtime-v8` ships and can restrict host access.

## Verification

**Implementation impact:** `apps/theme-sidecar/scripts/serve.mjs`, `apps/theme-sidecar/scripts/render-worker.mjs`, `README.md`, `docs/architecture/fastcart_ssr_architecture.md`.

**Verification criteria:**

- An infinite-loop theme returns a controlled 504 while `/health` remains responsive.
- A traversal asset path is rejected and never escapes the registered bundle.
- Response size and render timeout caps are covered by sidecar tests.
- The documented deployment binds the sidecar to loopback or a private network and requires a bearer secret off-loopback.

## Implementation status

Partial. The Node sidecar runs each render in a short-lived worker thread with
heap limits, a hard wall-clock deadline, a 10 MiB response cap and asset paths
confined to the registered bundle; it binds to loopback by default and refuses a
non-loopback bind without a bearer secret (`apps/theme-sidecar/scripts/serve.mjs`,
`apps/theme-sidecar/scripts/render-worker.mjs`). Theme registration and
activation are audited (`crates/api/src/routes/admin_theme.rs`). The sidecar does
not cap CPU time or total concurrent renders, and a worker can still use
filesystem and network APIs. The stronger isolate-level boundary remains future
work, so untrusted themes are out of scope.

This ADR is a decision, not a claim of completion.
