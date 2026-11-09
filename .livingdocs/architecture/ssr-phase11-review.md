---
type: Architecture Review
title: Universal SSR Runtime Phase 11 Review
description: Astro integration and adapter binding review.
kind: review
timestamp: 2026-11-09T12:00:00Z
---

# Universal SSR Runtime Phase 11 Review

## Scope

Phase 11 adds the second framework target:

- `templates/storefront-astro` uses Astro server output with
  `@astrojs/cloudflare`.
- `scripts/package.mjs` packages `dist/server` as the Worker and `dist/client` as
  the artifact client directory.
- `scripts/smoke.mjs` starts the artifact under workerd and checks SSR, trusted
  headers, streaming, cookies, redirects and errors.
- Astro's `/__fastcart/health` is implemented in `src/middleware.ts` because
  Astro excludes underscore-prefixed page directories from routing.
- The Rust workerd config generator now emits an `ASSETS` disk service binding
  for the artifact's client directory. Astro's Cloudflare entrypoint requires
  `env.ASSETS`, and the same binding is harmless for TanStack.
- CI job `astro-template` builds and smoke-tests the artifact.

## Acceptance checklist

| Criterion                                       | Status        | Evidence                                                                    |
| ----------------------------------------------- | ------------- | --------------------------------------------------------------------------- |
| Existing Astro storefront can adopt the runtime | Pass          | Template builds a standard Cloudflare adapter Worker                        |
| The adapter builds into a FastCart artifact     | Pass          | `dist/artifacts/<digest>/fastcart-ssr.json`                                 |
| The artifact starts under workerd               | Pass          | Smoke test reaches all fixture routes                                       |
| SSR works through `fastcart-serve`              | Pass (manual) | Server rendered the Astro page with the resolved store/deployment           |
| Trusted headers work                            | Pass          | `/echo` returns the injected store/deployment/request id                    |
| Streaming/cookies/redirects                     | Pass          | Smoke test covers all three                                                 |
| Static assets resolve                           | Pass          | Built with `assetsPrefix: /_fastcart/assets/`; Rust serves the client tree  |
| `env.ASSETS` is available                       | Pass          | Generated config binds a disk service named `ASSETS`                        |
| No Node-only API dependency                     | Pass          | Worker runs under workerd; no Node process                                  |
| Browser hydration is automated                  | Partial       | No browser harness for the Astro template                                   |
| Sessions/images bindings                        | Partial       | Adapter warns about `SESSION`/`IMAGES`; fixture routes do not use them      |
| Cloudflare-specific features match the contract | Partial       | Manifest still records capabilities only; binding declarations are Phase 12 |

## Findings

1. **Astro treats leading-underscore page directories as private.** The health
   route lives in middleware instead of `src/pages/__fastcart/health.ts`; public
   fixture routes remain normal pages.
2. **The adapter expects `ASSETS`.** Without the disk-service binding every
   request fails with `Cannot read properties of undefined (reading 'fetch')`.
   The generator now supplies the binding from the artifact client directory.
3. **Adapter bindings are not part of the artifact manifest.** Astro's default
   adapter enables Images and KV sessions; the fixture does not exercise them,
   and FastCart does not yet declare or provision those bindings.
4. **The Astro template has no client-side island yet.** There is no hydration
   test, but the SSR and asset paths match the TanStack contract.
5. **The `asssetsPrefix` spelling is Vite's/Astro's API.** Astro uses
   `build.assetsPrefix`; the generated URLs are checked manually.

## Verification

```bash
CI=true pnpm --filter @fastcart/storefront-astro build:artifact
CI=true pnpm --filter @fastcart/storefront-astro test:artifact
CI=true pnpm --filter @fastcart/storefront-astro typecheck
```

The server e2e used the same temporary config as Phase 5 and rendered the Astro
page at `shop.example.com`.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 11.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README lists the Astro template.

## Gate decision

Phase 12 (caching, preview and advanced behavior) may start. ASTRO and TanStack
share the artifact contract; framework-specific bindings are the remaining
integration gap.
