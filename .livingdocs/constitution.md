---
type: Constitution
title: Fastcart Constitution
description: Foundational scope, data model, and non-negotiables for the Fastcart commerce platform.
status: Draft
timestamp: 2026-11-06T16:11:01Z
---

# Fastcart Constitution

## Product

Fastcart is a lightweight, self-hosted commerce platform for independent stores: a single Rust modular monolith, one SQLite database per store, a React admin and a theme-based SSR storefront. It serves merchants who want Medusa-class domain capabilities without mandatory PostgreSQL, Redis or a message broker.

## Scope Boundaries

**In scope:**

- Store registry, domains, provisioning, lifecycle and multi-store isolation.
- Catalog, pricing, promotions, inventory, carts, checkout, orders, payments, fulfilment, returns and after-sales.
- Customers, addresses and store-scoped authentication.
- Theme registry, versioned activation/rollback, structured pages, previews and an SSR storefront.
- Admin operations, public storefront APIs, provider registries, webhooks, outbox, jobs and workflows.
- Single-node operations: SQLite, backups, metrics, health checks and a container image.

**Explicitly out of scope:**

- Microservices, Kubernetes-only deployment, mandatory PostgreSQL/Redis and distributed event streaming.
- Marketplace/multi-vendor operation, subscriptions and enterprise multi-region replication.
- Arbitrary untrusted user code execution inside the host process. Theme bundles are trusted; plugin code is not executed by the host.
- Fully autonomous AI commerce operations and a public plugin marketplace.
- Full storefront i18n, production data import and a plugin contribution system in the first release; these are separately scoped.

**Phase boundaries:**

- Phase 1: the commerce core, multi-store isolation, the admin operating surface, a complete default-theme purchase funnel, the Puck page editor and the embedded V8 SSR runtime with a switchable Node fallback.
- Phase 2: storefront i18n, data import, plugin contributions and AI-assisted editing.

## Data Model / Schema Foundation

```mermaid
erDiagram
    STORE ||--o{ STORE_DOMAIN : "owns"
    STORE ||--|| STORE_DATABASE : "isolated by"
    STORE_DATABASE ||--o{ PRODUCT : "contains"
    STORE_DATABASE ||--o{ CUSTOMER : "contains"
    STORE_DATABASE ||--o{ ORDER : "contains"
    PRODUCT ||--o{ VARIANT : "has"
    CART ||--o{ CART_ITEM : "contains"
    CART ||--o| CHECKOUT : "becomes"
    CHECKOUT ||--o| ORDER : "places"
    ORDER ||--o{ ORDER_ITEM : "snapshots"
    THEME ||--o{ PAGE : "renders"
```

The platform database owns the store registry, domains, users, sessions, memberships, roles, API tokens and provisioning jobs. Every store owns a separate SQLite database containing its catalog, pricing, inventory, carts, checkouts, orders, payments, content, themes, webhooks, jobs and outbox. There are no cross-database joins: a request resolves a store first, then opens that store's database, and every cache key is store-scoped. Published page snapshots are stored separately from draft bodies so a published page remains stable while editing continues. Theme code lives on the filesystem as trusted bundles; the database stores manifests, versions, settings and activation state.

## Non-negotiables

- Every storefront and admin request resolves exactly one store before any commerce access; an ID from another store is treated as not found. Verified by the cross-store isolation tests.
- Commerce rows exist only in the owning store's SQLite database; the platform database stores no catalog, cart, order or payment rows. Verified by the migration layout and isolation tests.
- Money is integer minor units. Totals, discounts, tax and shipping are calculated server-side; client-submitted totals are never trusted. Verified by cart, checkout and order tests.
- The production storefront serves only published records. Draft content requires a valid, signed, store-scoped preview token. Verified by storefront and preview tests.
- Theme bundles are trusted server-side code executed in a bounded runtime (worker thread or isolate) with CPU, heap and time limits. They are not an operating-system sandbox, and untrusted third-party code must not be loaded as a theme.
- Cookie-authenticated admin mutations require a CSRF token, and every handler enforces an RBAC permission before mutating data. Verified by auth and RBAC tests.
- A page body is validated against the active theme editor manifest before publication; an invalid page cannot be published. Verified by content and theme validation tests.
- Streaming responses carry a total deadline and byte cap; an aborted stream is never reported as a successful complete response. Verified by SSR and sidecar tests.
- Destructive store data removal requires a verified backup first. Verified by the platform deletion flow.
- A default deployment must run without Redis, PostgreSQL or a message broker; SQLite is the default and supported store.

## Amendment Log

Amendments are appended here as `## Amendment N — YYYY-MM-DD: summary`; the sections above are not edited once ratified.
