---
type: ADR
title: Single-node modular monolith with SQLite
description: Ship Fastcart as one Rust process with SQLite as the only required datastore.
owner: luozhouyang
status: Accepted
timestamp: 2026-11-06T16:17:43Z
---

# 0001. Single-node modular monolith with SQLite

## Context

Fastcart targets self-hosted independent stores that want Medusa-class commerce capabilities without operating a distributed system. The platform architecture sets mandatory PostgreSQL, Redis, Kubernetes and message brokers as explicit non-goals for the first release. The current codebase is a single Rust workspace with a platform database, one SQLite database per store, in-process events, a transactional outbox and a persistent SQLite job queue; the React admin and theme SSR are separate front ends over the same HTTP API.

A decision is required because adding a second database, a broker or a distributed scheduler would change deployment, failure modes and the minimum operator skill level for every future feature.

## Decision

We will ship Fastcart as one Rust modular monolith deployable as a single process image, with SQLite as the only required datastore: a platform database for the store registry, users, sessions and provisioning, and one SQLite database per store for commerce data. Events, webhooks, jobs and workflows run in-process against the store database and the outbox; no Redis, PostgreSQL or external broker is required for a supported deployment. The Node SSR sidecar is an optional companion process, not a mandatory service. Infrastructure boundaries stay behind traits so a PostgreSQL adapter can be added later without changing module APIs.

**Rejected alternatives:**

- **Microservices per commerce domain.** Rejected because the operational cost and cross-service consistency work dominate the value for single-node merchants.
- **One mandatory PostgreSQL database for every store.** Rejected as the default because SQLite keeps installation to one binary plus a data directory; PostgreSQL remains a future adapter.
- **Mandatory Redis for cache, locks or jobs.** Rejected because Moka, SQLite leases and the outbox cover the first release without another service.
- **An external message broker for events.** Rejected because the transactional outbox already gives durable at-least-once delivery from the same database transaction.

## Consequences

**Easier / gained:**

- Installation is a binary, a data directory and optional configuration; backups are per-store SQLite files.
- A request touches one process and at most two local databases, so failure modes and traces stay simple.
- Module boundaries remain testable with in-memory SQLite databases.

**Harder / accepted trade-offs:**

- SQLite write concurrency bounds one store's throughput; the platform scales vertically first.
- Cross-store reporting is not a single SQL query; it requires fan-out over store databases.
- A future PostgreSQL adapter must preserve the same store-scoped service contracts and transaction boundaries.

**Follow-ups:**

- Keep the PostgreSQL adapter as an infrastructure concern; do not leak SQLite-specific SQL into module domains.
- Document the single-node deployment limits and the scale-up path.

## Verification

**Implementation impact:** `Cargo.toml`, `apps/server`, `crates/infrastructure`, `crates/api`, `Dockerfile`, `docker-compose.yml`.

**Verification criteria:**

- A clean deployment starts from the documented image with only the platform database, a store directory and optional sidecar configuration.
- `cargo test --workspace` passes without a PostgreSQL or Redis service.
- The isolation suite provisions two stores and observes no shared commerce rows.
- The Docker health check reaches `/health/ready` with no external datastore.
