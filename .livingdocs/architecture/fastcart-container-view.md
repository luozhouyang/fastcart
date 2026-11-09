---
type: Architecture View
title: Fastcart Container View
description: Container-level map of the admin, storefront, Rust API, SQLite, universal SSR gateway and workerd runtime.
kind: container
timestamp: 2026-11-09T02:46:06Z
---

# Fastcart Container View

## Orientation

This container view shows the target universal SSR topology: how a browser
request reaches the Rust API, the platform and per-store databases, the SSR
gateway, the `workerd` runtime and external providers. It is for engineers
changing deployment topology, crate boundaries or the SSR runtime.

The universal runtime is specified by
[ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) and the phased
[Universal SSR Runtime](../architecture/universal-ssr-runtime.md) plan. The Node
sidecar and `runtime-v8` are legacy and are not part of this target.

```mermaid
flowchart TB
    Browser[Browser] --> Admin[React Admin]
    Browser --> Storefront[Framework Storefront]
    Admin -->|JSON, cookies, CSRF| API[Rust Axum API]
    Storefront -->|public request| API

    API -->|registry, auth, provisioning| PlatformDB[(Platform SQLite)]
    API -->|one pool per store| StoreDB[(Per-store SQLite)]
    API -->|static assets| Assets[Static Asset Server]

    API -->|trusted request| Gateway[SSR Gateway]
    Gateway -->|loopback HTTP| Workerd[workerd Process]
    Workerd -->|Fetch Handler| Artifact[Storefront Artifact]
    Artifact -->|Commerce API| API

    Manager[SSR Runtime Manager] -->|config, restart, health| Workerd
    Manager --> Registry[Deployment Registry]
    Manager --> ArtifactStore[Artifact Storage]
    API -->|outbox, jobs, webhooks| StoreDB
    API -->|asset blobs| Storage[(Local object storage)]
    API -->|payment, email, webhooks| Providers[External providers]
```

## Boundaries

- `fastcart-serve` is the only public HTTP entry point. It resolves the store
  from the trusted hostname, opens the store database and owns commerce,
  authentication, admin and static asset serving.
- The SSR gateway strips client-supplied internal headers and injects a trusted
  store/deployment/request context. The Worker cannot choose a store.
- `workerd` executes framework-built Worker artifacts and calls the internal
  Commerce API. It never receives database access, admin credentials or payment
  secrets.
- `fastcart-ssr` owns the runtime manager, process supervisor, artifact
  validation and deployment lifecycle. A workerd process crash must not take
  down the commerce API.
- Static assets under `/_fastcart/assets/` are served by Rust, not by workerd.

## Implementation status

The workerd runtime, artifact pipeline, gateway and runtime manager are not
implemented yet. The current repository still runs the legacy Node sidecar
(`apps/theme-sidecar`) and keeps `runtime-v8` as a stub. This view describes the
target container boundary, not the current deployment.

## Drift instrument

The target boundary is exercised by four suites: cross-store isolation, artifact
validation (manifest, digest, path traversal, missing entry), SSR gateway tests
(header filtering, trusted context, streaming, timeouts, multiple `Set-Cookie`)
and workerd framework fixtures (TanStack Start, Astro, generic Fetch handler).
A dependency from `crates/kernel` or `crates/storefront` onto workerd-specific
types is a boundary drift signal; keep workerd process management inside
`fastcart-ssr`. The legacy sidecar protocol tests are transitional and must be
removed with the legacy adapters.
