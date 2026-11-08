---
type: ADR
title: Theme bundle as the product storefront
description: The theme bundle and Rust SSR pipeline are the storefront; the TanStack app is a harness.
owner: luozhouyang
status: Accepted
implementation: partial
timestamp: 2026-11-06T16:17:43Z
---

# 0005. Theme bundle as the product storefront

## Context

The original platform plan described a TanStack Start storefront application with routes for home, product, collection, search, cart, checkout and account. The implemented storefront is instead a trusted theme bundle rendered by the Rust API through the Node/V8 SSR runtime, with a theme registry, editor manifest, dynamic product/collection routes and page bindings. `apps/storefront` remains a TanStack Start shell with a placeholder home route and is not part of the production rendering path.

A decision is required because two storefront surfaces create ambiguity about where the purchase funnel belongs, which one docs and tests should describe, and whether a second SSR runtime must be supported.

## Decision

We will treat the theme bundle plus the Rust SSR pipeline as the product storefront. The default theme owns the customer-facing routes and components, including the purchase funnel, and is versioned, activated and rolled back through the theme registry. `apps/storefront` is a development harness for experimenting with TanStack Start and is not shipped or required in production; new production routes and commerce UI go into theme components and the theme SDK. The storefront SDK remains the shared data-access layer for both.

**Rejected alternatives:**

- **Make the TanStack Start app the product storefront.** Rejected because it duplicates the theme registry, editor and SSR pipeline the platform already built and would leave theme authors without a supported route model.
- **Ship both surfaces as supported products.** Rejected because it doubles the routing, SSR and testing matrix and confuses which surface owns browser hydration.
- **Delete `apps/storefront` immediately.** Rejected for now because it remains useful as a local experiment; it is explicitly non-production and can be retired once the theme harness replaces it.

## Consequences

**Easier / gained:**

- One storefront contract: themes, SSR and the theme editor describe the same routes.
- The purchase funnel can reuse theme components and the storefront SDK without a second router.
- Documentation and tests have a single surface of record.

**Harder / accepted trade-offs:**

- Theme authors must implement routes and commerce UI inside the themed component model rather than a general-purpose app router.
- The unused TanStack app can drift; it must stay clearly marked or be removed.
- Route-level code splitting and framework features remain the theme build's responsibility.

**Follow-ups:**

- Implement the purchase funnel in the default theme.
- Mark `apps/storefront` as a non-production harness in its README and the repository layout.
- Update the platform design docs that still describe TanStack Start as the product.

## Verification

**Implementation impact:** `themes/default`, `packages/theme-sdk`, `packages/storefront-sdk`, `apps/storefront`, `docs/architecture/fastcart_ssr_architecture.md`.

**Verification criteria:**

- The storefront end-to-end test runs against a theme bundle and does not start `apps/storefront`.
- The default theme renders every documented customer route without a TanStack Start server.
- The repository layout and README identify `apps/storefront` as a harness, not a production surface.

## Implementation status

Partial. Theme bundles are registered, versioned, activated and rolled back per
store, and the default theme renders through the theme runtime with dynamic
product and collection routes (`crates/modules/theme`,
`crates/api/src/routes/admin_theme.rs`, `themes/default/theme.json`,
`themes/default/src/server.tsx`). `apps/storefront` is a non-production
development harness and is marked as such in its README. The default theme has
no cart, checkout, payment, account or search UI, so the documented purchase
funnel is not complete.

This ADR is a decision, not a claim of completion.
