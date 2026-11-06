---
type: Architecture View
title: Fastcart Container View
description: Container-level map of the admin, storefront, Rust API, databases, SSR sidecar and external providers.
kind: container
timestamp: 2026-11-06T16:11:48Z
---

# Fastcart Container View

## Orientation

This container view shows how a browser request reaches the Rust API, the platform and per-store databases, the Node SSR sidecar and external providers. It is for engineers changing deployment topology, crate boundaries or the SSR runtime.

```mermaid
flowchart TB
    Browser[Browser] --> Admin[React Admin]
    Browser --> Storefront[Theme Storefront]
    Admin -->|JSON, cookies, CSRF| API[Rust Axum API]
    Storefront -->|SSR request| API
    API -->|registry, auth, provisioning| PlatformDB[(Platform SQLite)]
    API -->|one pool per store| StoreDB[(Per-store SQLite)]
    API -->|render payload| Sidecar[Node SSR sidecar]
    Sidecar -->|trusted bundle| Theme[Theme bundle files]
    API -->|outbox, jobs, webhooks| StoreDB
    API -->|asset blobs| Storage[(Local object storage)]
    API -->|payment, email, webhooks| Providers[External providers]
```

## Drift instrument

The container boundary is exercised by the cross-store isolation suite, the storefront SSR integration tests and the sidecar protocol tests. A dependency from `crates/kernel` or `crates/storefront` onto a provider crate is a boundary drift signal; review crate manifests when adding dependencies and run `cargo test --workspace`. The sidecar contract is covered by `node --test apps/theme-sidecar/test`.
