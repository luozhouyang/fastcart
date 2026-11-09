---
type: Architecture Review
title: Universal SSR Runtime Phase 7 Review
description: Rust static asset serving and hybrid rendering review.
kind: review
timestamp: 2026-11-09T08:00:00Z
---

# Universal SSR Runtime Phase 7 Review

## Scope

Phase 7 adds Rust-owned static asset serving for universal artifacts:

- `GET /_fastcart/assets/{*path}` reads from the active artifact's
  `assets.directory` (normally `client/`).
- The handler is global, not store-resolved; the artifact is process-scoped in
  this phase.
- Canonicalization plus a component check rejects traversal and symlink escapes.
- Responses carry a content type, `Content-Length` and
  `Cache-Control: public, max-age=31536000, immutable`.
- The TanStack template sets Vite `base` to `/_fastcart/assets/` while keeping
  the router basepath at `/`; the generated document references that prefix.

## Acceptance checklist

| Criterion                          | Status  | Evidence                                                                         |
| ---------------------------------- | ------- | -------------------------------------------------------------------------------- |
| Static assets are served by Rust   | Pass    | `UniversalSsr::serve_asset` plus `/_fastcart/assets/{*path}` test                |
| Correct content types              | Pass    | JS/CSS/JSON/images/fonts mapping; real JS asset returned as `text/javascript`    |
| Traversal is blocked               | Pass    | `../fastcart-ssr.json` returns 404 in Rust and through the server                |
| Missing files return 404           | Pass    | Unit test and route test                                                         |
| Cache headers are safe             | Pass    | Immutable one-year cache; artifact content is digest-addressed                   |
| Hybrid rendering falls back to SSR | Pass    | All non-asset paths still use the storefront handler/gateway                     |
| TanStack hydration assets resolve  | Pass    | Server e2e returned the 343 KB client bundle from `/_fastcart/assets/assets/...` |
| Browser hydration is automated     | Partial | Assets and document match; no Playwright test yet                                |
| Range requests and compression     | Not yet | Phase 12                                                                         |
| Per-store artifact selection       | Not yet | Phase 8                                                                          |
| Stale artifact cleanup             | Not yet | Each imported digest remains on disk; GC is Phase 9                              |

## Findings

1. **Vite base and TanStack router base are separate.** Setting Vite
   `base: "/_fastcart/assets/"` alone made TanStack redirect `/` to the asset
   prefix because the plugin derived the router basepath from Vite's base.
   Setting `tanstackStart({ router: { basepath: "/" } })` keeps page routes at
   `/` while asset URLs use the prefix.
2. **The asset route is deliberately not store-aware.** One process has one
   active artifact; multi-store routing must add a deployment/artifact selector
   before this route can serve different stores.
3. **No conditional requests.** The response does not yet emit `ETag` or handle
   `If-None-Match`; immutable URLs make that less urgent but Phase 12 should add
   it for non-hashed files.
4. **The server e2e remains manual.** Unit tests cover path handling and route
   wiring; the full `fastcart-server` run is documented rather than automated.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets_are_served_without_store_resolution
CI=true pnpm --filter @fastcart/storefront-tanstack build:artifact
CI=true pnpm --filter @fastcart/storefront-tanstack test:artifact
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 7.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the asset route and template base.

## Gate decision

Phase 8 (multi-store deployment registry) may start. Static assets are now
usable for the single active artifact; per-store selection and cleanup remain
open.
