---
type: ADR
title: Trusted theme bundles with worker limits
description: Treat storefront artifacts as trusted server-side code isolated by workerd process groups and resource limits, not an OS sandbox.
owner: luozhouyang
status: Accepted
implementation: partial
timestamp: 2026-11-06T16:17:43Z
---

# 0003. Trusted theme bundles with worker limits

## Context

Storefront artifacts execute server-side and therefore run code with access to
whatever the SSR runtime exposes. The platform excludes arbitrary untrusted
server-side code from its first release. Registration and activation are
admin-only operations, so a storefront artifact is closer to an installed
server plugin than to user content.

The universal SSR runtime runs artifacts inside workerd, a process-isolated
JavaScript/Wasm runtime. A workerd isolate is a reliability and execution
boundary, not a hardened security boundary. The project explicitly warns that
workerd is not a hardened sandbox. The current legacy Node sidecar uses worker
threads with heap and wall-clock limits, but it remains transitional and is
being replaced under [ADR 0007](0007-universal-ssr-runtime-with-workerd.md).

A decision is required because calling an isolate a "sandbox" would set a
security expectation the implementation does not meet, while refusing to run
storefront artifacts at all would remove the product's rendering model.

## Decision

We will treat storefront artifacts as trusted server-side code and isolate them
with workerd process groups plus explicit runtime limits.

- Artifacts run in a workerd process managed by `fastcart-ssr`; each runtime
  group is a separate OS process, and trusted artifacts may share a group.
- workerd binds to loopback or a private network only. The gateway authenticates
  every internal request and injects trusted store and deployment context; the
  Worker never receives database access, payment secrets, admin credentials or
  other backend secrets.
- Workspace filesystem access is limited to the artifact directory. Static
  assets are served by Rust, not by the Worker. Outbound network access follows
  an explicit per-group policy rather than unrestricted host networking.
- The supervisor enforces bounded restarts with backoff, request and header
  timeouts, idle-body timeouts, concurrency limits, artifact-size limits and
  process resource limits where the OS supports them.
- A workerd isolate is not claimed as an OS security boundary. Unreviewed
  third-party or merchant-uploaded code requires a dedicated process group and
  an OS-level or VM-level boundary.
- Registering and activating an artifact is a privileged admin operation and
  must be audited.

**Rejected alternatives:**

- **Node worker threads as the trust boundary.** Rejected because a worker can
  still use Node filesystem and network APIs, and the legacy sidecar does not
  cap CPU or global concurrency. It is a reliability boundary, not a security
  boundary.
- **An embedded V8 adapter as the stronger boundary.** Rejected under ADR 0007;
  maintaining a JavaScript runtime in-process does not provide OS isolation
  either, and it does not accept standard framework artifacts.
- **A container, VM or child process per render.** Rejected for v1 because the
  lifecycle cost is disproportionate for trusted artifacts; a dedicated runtime
  group per trust domain is the supported isolation unit.
- **WASM-only storefronts.** Rejected because the supported frameworks and their
  server bundles cannot be expressed that way without a rewrite.

## Consequences

**Easier / gained:**

- A crashed or runaway artifact is contained by its workerd process group and
  can be restarted without restarting the commerce API.
- The trust model is explicit: trusted artifacts share a runtime group;
  untrusted code requires a stronger boundary.
- Store context and commerce data stay behind the Rust API instead of being
  exposed to JavaScript directly.

**Harder / accepted trade-offs:**

- A malicious trusted artifact can still consume CPU, memory and network within
  its group's policy; the operator must configure limits and egress.
- Process groups add lifecycle, health-check and log-management work.
- Untrusted artifact hosting remains out of scope until an OS/VM isolation layer
  is added.

**Follow-ups:**

- Implement runtime-group configuration and resource limits in `fastcart-ssr`.
- Document per-group egress policy and filesystem permissions.
- Retire the legacy Node sidecar trust guidance once workerd reaches parity.

## Verification

**Implementation impact:** `crates/fastcart-ssr`, `apps/server`, workerd runtime
configuration, artifact validator, runtime supervisor, `README.md`,
`.livingdocs/architecture/universal-ssr-runtime.md`.

**Verification criteria:**

- A crashed or timed-out worker returns a controlled response while
  `fastcart-serve` stays healthy.
- An artifact cannot read store databases, admin configuration, payment secrets
  or another store's artifact directory.
- Internal context headers are stripped and re-injected by the gateway; a
  spoofed store or deployment header is rejected.
- Runtime restart backoff and health transitions are covered by tests.
- The workerd listener is loopback/private and internal requests require a
  token.
- The documentation states that a workerd isolate is not a hardened sandbox and
  defines the isolation level required for untrusted artifacts.

## Implementation status

Partial. The legacy Node sidecar still runs each render in a short-lived worker
thread with heap limits, a hard wall-clock deadline and a 10 MiB response cap,
binds to loopback by default, and refuses a non-loopback bind without a bearer
secret (`apps/theme-sidecar/scripts/serve.mjs`,
`apps/theme-sidecar/scripts/render-worker.mjs`). The workerd runtime groups,
artifact isolation, egress policy and OS-level limits described above are not
implemented. The Node sidecar is transitional and is being replaced under ADR 0007.

This ADR is a decision, not a claim of completion.
