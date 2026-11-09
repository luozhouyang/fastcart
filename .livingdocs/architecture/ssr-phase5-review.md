---
type: Architecture Review
title: Universal SSR Runtime Phase 5 Review
description: TanStack Start template, artifact build and end-to-end SSR review.
kind: review
timestamp: 2026-11-09T06:00:00Z
---

# Universal SSR Runtime Phase 5 Review

## Scope

Phase 5 adds the first maintained framework template:

- `templates/storefront-tanstack` — TanStack Start + React + Vite +
  `@cloudflare/vite-plugin` starter.
- `scripts/package.mjs` — copies the Vite server/client outputs into the
  artifact layout and calls `@fastcart/storefront-build`.
- `scripts/smoke.mjs` — starts the packaged artifact under workerd and checks
  SSR, trusted headers, streaming, duplicate cookies, redirects and errors.
- CI job `tanstack-template` builds the artifact, downloads pinned workerd and
  runs the smoke test.

Routes: `/`, `/__fastcart/health`, `/echo`, `/stream`, `/cookie`, `/redirect`,
`/boom`.

## Acceptance checklist

| Criterion                                       | Status        | Evidence                                                                                                                      |
| ----------------------------------------------- | ------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| A new TanStack storefront can be scaffolded     | Pass          | Template package with `pnpm dev` and `build:artifact`                                                                         |
| The application builds into a FastCart artifact | Pass          | `dist/artifacts/<digest>/fastcart-ssr.json`                                                                                   |
| The artifact can be deployed                    | Pass          | Packaged artifact starts under workerd in `smoke.mjs`                                                                         |
| SSR works through `fastcart-serve`              | Pass (manual) | A server with `[ssr]` rendered the template at `shop.example.com`; trusted `/echo` returned the resolved store and deployment |
| Static assets work                              | Partial       | Vite emits `dist/client`; Rust asset serving is Phase 7                                                                       |
| Client hydration works                          | Partial       | SSR document includes the hydration script; the asset URL is not served by Rust yet                                           |
| No Node-only runtime dependency                 | Pass          | The packaged Worker runs under workerd without a Node process                                                                 |
| Loaders/server functions                        | Pass          | `/` uses `createServerFn` in the route loader                                                                                 |
| Cookies                                         | Pass          | `/cookie` preserves two `Set-Cookie` headers through the gateway                                                              |
| Redirects                                       | Pass          | `/redirect` returns 302                                                                                                       |
| Streaming                                       | Pass          | `/stream` produces multiple chunks; the server log shows a 303 ms stream                                                      |
| Errors                                          | Pass          | `/boom` returns a controlled response and does not crash the runtime                                                          |
| Concurrency                                     | Not tested    | Phase 8/load tests                                                                                                            |

## Findings

1. **Static assets are the remaining blocker for hydration.** The TanStack
   document references `/assets/*`; the universal gateway does not serve the
   artifact's client directory yet. Phase 7 adds `/_fastcart/assets/` (or an
   equivalent deployment asset route) and rewrites the template's asset base.
2. **Route escaping required `[_]`.** A literal `__fastcart` segment is treated
   as a pathless route by the TanStack generator; the health route file uses
   `[_]_fastcart.health.ts` to keep `/__fastcart/health` literal.
3. **The template is pinned to the mirror's available TanStack version.**
   `@tanstack/react-start` `1.169.5` is used because `1.169.6` was not yet
   available from the configured registry. The Worker contract is unchanged.
4. **The end-to-end server run is manual.** The smoke test covers workerd; the
   `fastcart-serve` path was validated with a temporary config and a provisioned
   store, but is not yet a committed test. Phase 7/8 should add a server-level
   fixture.
5. **No env binding contract yet.** The template does not exercise Cloudflare
   bindings; the artifact manifest has only a capability allowlist. Phase 8
   defines which bindings FastCart supplies.

## Verification

```bash
CI=true pnpm --filter @fastcart/storefront-tanstack build:artifact
CI=true pnpm --filter @fastcart/storefront-tanstack test:artifact
CI=true pnpm --filter @fastcart/storefront-tanstack typecheck
```

The smoke test skips when the pinned workerd binary is unavailable.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 5.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README repository layout lists the template.

## Gate decision

Phase 6 (Storefront SDK and Commerce integration) may start. Phase 7 must add
static asset serving before the template is considered hydration-complete.
