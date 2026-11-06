---
type: ADR
title: Per-store SQLite database for tenant isolation
description: Each store owns a SQLite database and pool; the platform database holds registry and auth data only.
owner: luozhouyang
status: Accepted
timestamp: 2026-11-06T16:17:43Z
---

# 0006. Per-store SQLite database for tenant isolation

## Context

Fastcart is multi-store by design. A single shared database with a `store_id` column on every table would make cross-store leaks a query-authoring mistake away, force every index and cache key to carry the tenant, and make per-store backup, restore and deletion difficult. The platform architecture instead specifies one SQLite database per store, with the platform database holding only the store registry, domains, users, sessions, memberships and provisioning state.

A decision is required because tenancy is the security boundary every commerce service, cache and background job must respect; changing it later would rewrite migrations, repositories and the isolation test suite.

## Decision

We will enforce tenancy with one SQLite database and connection pool per store. The platform database never stores catalog, cart, order, payment or content rows. A request resolves exactly one store from trusted platform metadata (the host on storefront requests, the authenticated membership plus `x-store-id` on admin requests), opens that store's database, and passes a `StoreDatabase` to every commerce service. Background jobs, outbox rows, workflows and per-request caches are store-scoped; cache keys include the store ID. A store's data is backed up, restored and deleted as that store's database.

**Rejected alternatives:**

- **A shared database with row-level `store_id` filtering.** Rejected because a single missing predicate leaks another merchant's data and because per-store backup and deletion become complex.
- **A schema per store in PostgreSQL.** Rejected as the default because it still requires PostgreSQL and complicates migration tooling; a future PostgreSQL adapter can preserve the per-store boundary with separate databases.
- **One database per store but with cross-store joins for analytics.** Rejected for the request path; analytics may fan out explicitly but must never join commerce tables across stores.

## Consequences

**Easier / gained:**

- The database boundary is the tenancy boundary; a missing filter cannot read another store's rows.
- Per-store backup, restore, export and deletion are file-level operations.
- Tests can prove isolation by opening two databases and asserting no shared handles.

**Harder / accepted trade-offs:**

- Every store carries migration, pool and connection overhead.
- Cross-store reporting requires fan-out and aggregation outside SQL.
- The platform database and store databases must be provisioned atomically enough to avoid orphaned stores.

**Follow-ups:**

- Keep every new commerce table in a store migration, never a platform migration.
- Keep cache keys store-scoped and cover them in the isolation suite.
- Document the PostgreSQL adapter as a future implementation of the same boundary.

## Verification

**Implementation impact:** `migrations/platform`, `migrations/store`, `crates/infrastructure/src/store_db_manager.rs`, `crates/http/src/middleware.rs`, `crates/api/src/state.rs`, all commerce modules.

**Verification criteria:**

- The isolation suite creates two stores and asserts that admin and storefront requests for one store cannot read or mutate the other's catalog, cart or orders.
- A store migration runner applies only to that store's database.
- Cache entries and job leases are scoped by store ID.
- Backup and restore operate on a single store database without touching another store.
