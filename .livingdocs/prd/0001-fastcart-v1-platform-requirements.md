---
type: PRD
title: Fastcart v1 Platform Requirements
description: Product requirements for the Fastcart v1 self-hosted multi-store commerce platform.
status: Draft
timestamp: 2026-11-06T16:11:01Z
---

# 0001. Fastcart v1 Platform Requirements

## Problem / Motivation

Independent merchants who want Medusa-class commerce capabilities either accept a multi-service deployment (PostgreSQL, Redis, workers) or assemble a store from disconnected tools. Fastcart's thesis is a single Rust modular monolith with one SQLite database per store, a React admin and a framework-agnostic SSR storefront, deployable on one node.

The backend already implements a broad commerce domain, but the product is not yet operable end to end: the default theme has no cart or checkout, the admin exposes only a subset of the available APIs, several admin settings are not enforced by the backend, and the planned i18n, import, plugin and AI work has no durable product specification. This PRD pins the v1 scope, the explicit non-goals, and the conditions that prove the platform is sellable.

## Who asked

Fastcart maintainers and self-hosted independent-store operators. The repository owner requested this PRD as the product baseline for the platform.

## Goals

- A newly provisioned store can complete a real purchase on the default theme without custom code: browse, add to cart, check out, pay, receive confirmation and see the order in Admin.
- An owner can perform the core commerce operations listed in the requirements from the Admin UI, without direct API calls.
- A single-node deployment needs only the Rust server, SQLite and, when SSR is enabled, a process-isolated `workerd` runtime. No Redis, PostgreSQL, message broker or Docker is mandatory.
- Store isolation, published-content safety and preview-token scoping are enforced and covered by repeatable tests.
- Extension points (provider registries, webhooks, workflows, themes) have stable contracts, even where the first built-in implementations are intentionally small.

## Non-goals

- Microservices, Kubernetes, mandatory PostgreSQL/Redis, distributed event streaming and distributed job scheduling.
- Marketplace or multi-vendor functionality, subscriptions, and enterprise multi-region replication.
- Arbitrary untrusted server-side code execution. Storefront artifacts remain trusted code; plugins do not execute inside the host process, and a `workerd` isolate is not an OS sandbox.
- Full storefront i18n, a production data-import pipeline, a plugin contribution marketplace and autonomous AI operations in v1. These are separately scoped and must not silently expand this PRD.
- Replacing the current block editor with Puck in v1. The versioned `PageBody` remains the canonical schema, and the migration is tracked as next-scope work in [Issue 0002](../issues/0002-migrate-the-admin-page-editor-to-puck.md); it is not a v1 acceptance criterion.

## Requirements

- **Platform and tenancy.** Every storefront and admin request resolves exactly one store; a record from another store is never readable or mutable; each store's commerce data lives in its own SQLite database. Verified by the cross-store isolation suite.
- **Catalog and pricing.** Products support variants, options, images, categories, collections, tags, sales channels and publish state. Prices are integer minor units, calculated server-side, and deterministic under price lists, customer groups and promotions. Verified by catalog, pricing and promotion tests.
- **Inventory and fulfillment.** Stock is tracked by location with reservations, commits, releases and movement history; fulfilment records are persisted and inventory-side effects are idempotent. Verified by inventory and after-sales tests.
- **Cart, checkout and orders.** Carts support guest and authenticated customers with server-derived totals. Checkout is a persisted state machine whose completion is idempotent. Orders store immutable commercial snapshots. Returns, claims, exchanges, order edits and refunds are supported. Verified by workflow and API tests.
- **Payments.** A provider interface supports manual and Stripe payment flows with sessions, captures, refunds, signed webhooks and idempotent event handling. A provider failure must not leave a paid order or double-count inventory.
- **Storefront.** Storefront artifacts are versioned, validated, activated and rolled back per store. SSR runs in a process-isolated `workerd` runtime and streams HTML; the Rust server injects trusted store context and never exposes database or backend secrets to the Worker. Framework builds (TanStack Start first, Astro second) produce the standard Fetch-handler artifact; the runtime is framework-agnostic. SSR serves published records only. Dynamic product and collection routes resolve published resources. Draft content is reachable only through signed, revocable preview tokens. Sitemap and robots reflect the active routes. The default storefront must implement the complete purchase funnel named in the Goals.
- **Admin.** The core resources listed in `Acceptance criteria` are manageable from the Admin UI with permission-aware actions, loading/empty/error states and CSRF protection. Page editing keeps the versioned `PageBody` as the canonical schema; the Puck migration is tracked by [Issue 0002](../issues/0002-migrate-the-admin-page-editor-to-puck.md) as next-scope work and is not a v1 acceptance criterion. Settings that the UI exposes must be read and enforced by the backend.
- **Content and SEO.** Pages have revisions, draft and published bodies, preview and publish workflows, navigation and redirects. Each rendered page exposes title, description, canonical, Open Graph and Twitter metadata; the sitemap carries `<lastmod>`; dynamic routes expose a canonical URL.
- **Extensibility.** Provider registries, webhook subscriptions with retries and replay, a transactional outbox, a persistent job queue and a durable workflow engine are available. The workflow engine is a tested foundation and is not yet wired into the core checkout path; the misleading empty Admin workflow-inspection routes were removed. Theme capability grants are scoped and short-lived.
- **Security and operations.** Admin authentication uses hashed sessions or API tokens with RBAC; cookie mutations require CSRF; login and public writes are rate-limited; request and response sizes are capped; security headers and request IDs are present; audit records cover privileged mutations. Backups can be created and verified.
- **Deployment.** The documented deployment is a non-root `fastcart-serve` binary plus a pinned, supervised `workerd` process when SSR is enabled. Configuration is TOML plus environment overrides. Docker is not a supported deployment path. The admin hosting strategy, SSR gateway, runtime health and rollback are documented.

## Acceptance criteria

- A visitor on a freshly provisioned store completes browse → add to cart → checkout → payment → order confirmation on the default theme, and the resulting order is visible in Admin with the correct totals and inventory movement.
- An owner can create a product with variants and prices, create a promotion, configure a region, shipping option and tax rate, publish a content page and edit a theme setting entirely from Admin.
- The cross-store isolation suite reports zero reads or writes across stores; storefront SSR tests report zero draft or archived records without a valid preview token.
- `cargo test --workspace`, clippy, `pnpm lint`, `pnpm typecheck`, `pnpm build` and `pnpm test` pass, including the SSR artifact, gateway and workerd framework-fixture tests.
- A single-node deployment starts from the documented binary and supervised `workerd` process, passes `/health/ready`, serves the admin application and renders a streaming storefront page.
- A store backup can be created and verified, and a restore is documented and exercised on a disposable store database.

## Success metrics

- Time to first sale from a clean install is under 30 minutes of operator work, measured by a scripted dry run.
- 100% of the core tasks in the acceptance criteria are completed through the Admin UI, with zero direct API calls.
- Cross-store leak count is zero in the isolation suite, and published-content leak count is zero in the SSR suite.
- The default theme's purchase funnel has a deterministic end-to-end test that completes once per CI run.
- The storefront render path enforces its configured timeout and returns a controlled error on expiry, verified by integration tests.

## Current implementation baseline

As of 2026-11-09, the Rust backend covers the commerce modules, multi-store isolation, provider registries, outbox/jobs, workflows, webhooks, theme lifecycle, streaming SSR, dynamic routes, previews and sitemap `lastmod`. Admin defines 18 routes with permission-aware navigation and read guards; the routes are: overview, orders, products (variant and base-price authoring), inventory, customers, promotions/coupons, forms, inbox, notifications, themes, pages, settings, regions (with publishable API keys), shipping (profiles/options/rates) and tax (categories/regions/rates). Admin navigation and screen guards now hide sections and disable write actions the signed-in account cannot use server-side. Page editing still uses the bespoke editor; Puck is not adopted. The default theme browses products and collections and includes a client-side cart/checkout drawer with manual payment and order confirmation; it still has no account or search UI. Storefront SSR still runs through the legacy Node sidecar; the universal workerd runtime is accepted but not implemented. Region-scoped carts now carry a resolved region, enforce its currency and destination countries, constrain checkout payment providers, and pin tax calculation to the region's tax region. Checkout also enforces the Admin's guest-checkout, required-phone and confirmation-email settings. Payment and email provider registries can be scoped per store through environment overrides. Public writes use trusted-proxy-aware client-IP rate limits. i18n, data import, plugin contributions and AI execution are not implemented.

## Open questions

- Region-driven currency, country validation, tax region and payment-provider selection are v1 requirements and are implemented; the store-level currency remains the fallback when no region applies.
- The full Phase 12 Admin scope is v1: variants/prices, promotions, regions, shipping and tax screens are implemented, so "without direct API calls" is the acceptance bar.
- The universal workerd SSR runtime is the v1 target architecture (ADR 0007). It is not implemented yet; the legacy Node sidecar is transitional and must not be extended. Puck remains post-v1, and the bespoke page editor is the supported v1 implementation (ADR 0004).
- Are gift cards, draft orders, product types and a return-reason catalogue v1 requirements or later parity work?
- Does the plugin system need a working contribution/capability vertical slice in v1, or is the manifest foundation sufficient?

## Related

- Constitution: [constitution.md](../constitution.md)
- ADRs: [0001](../adr/0001-single-node-modular-monolith-with-sqlite.md), [0002](../adr/0002-embedded-v8-ssr-with-a-switchable-node-fallback.md) (superseded by 0007), [0003](../adr/0003-trusted-theme-bundles-with-worker-limits.md), [0004](../adr/0004-puck-as-the-admin-page-editor.md), [0005](../adr/0005-theme-bundle-as-the-product-storefront.md) (superseded by 0007), [0006](../adr/0006-per-store-sqlite-database-for-tenant-isolation.md), [0007](../adr/0007-universal-ssr-runtime-with-workerd.md).
- Source design notes (local-only, not versioned): `docs/architecture/fastcart_platform_architecture.md`, `fastcart_platform_implementation.md`, `fastcart_ssr_architecture.md`, `theme-editor-plugin-ai-roadmap.md`, `docs/data-import-architecture.md`, `docs/storefront-i18n-architecture.md`.
- Issues: [0001 — embedded V8 SSR](../issues/0001-implement-embedded-v8-ssr-with-runtime-switching.md) (superseded by 0003), [0002 — Puck editor](../issues/0002-migrate-the-admin-page-editor-to-puck.md), [0003 — universal SSR runtime](../issues/0003-implement-universal-ssr-runtime-with-workerd.md). More issues are opened as requirements enter implementation.
