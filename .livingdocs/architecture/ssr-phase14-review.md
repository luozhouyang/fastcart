---
type: Architecture Review
title: Universal SSR Runtime Phase 14 Review
description: Route-level SSR cache and invalidation review.
kind: review
timestamp: 2026-11-09T15:00:00Z
---

# Universal SSR Runtime Phase 14 Review

## Scope

Phase 14 adds opt-in route-level caching:

- A Worker marks a response cacheable with `Cache-Control: public, max-age` or
  `s-maxage`.
- Rust buffers the response (up to 2 MiB) and serves later cookie-free,
  authorization-free `GET`s from an in-memory store.
- Cache keys include store, path/query and `Accept-Language`.
- `Set-Cookie`, `private`, `no-store`, and `Vary: Cookie`/`Authorization`
  responses are never cached.
- `x-fastcart-cache` reports `HIT`/`MISS`; `Age` is included on hits.
- Admin/store mutations and deployment activation invalidate the store's cache.
- Preview requests bypass the cache.

## Acceptance checklist

| Criterion                             | Status  | Evidence                                                                     |
| ------------------------------------- | ------- | ---------------------------------------------------------------------------- |
| Worker controls route caching         | Pass    | `Cache-Control` metadata is parsed; no header means `no-store`               |
| Cache hit serves a buffered response  | Pass    | `/cached` worker counter stays constant on the second request                |
| Invalidation works                    | Pass    | Explicit invalidation triggers a new worker response                         |
| Admin mutations invalidate            | Pass    | Audit middleware calls the runtime invalidation hook after successful writes |
| Storefront mutations invalidate       | Pass    | Context-invalidation middleware calls the hook for carts/customers           |
| Personal data is not cached           | Pass    | Cookie/auth requests bypass; `Set-Cookie`/private responses are rejected     |
| Preview bypasses cache                | Pass    | Preview path uses `render_preview`, which has no cache reads                 |
| Stale-while-revalidate                | Not yet | Expired entries are removed and refetched synchronously                      |
| Persistent/shared cache               | Not yet | Cache is per-process memory                                                  |
| Cache tags / per-product invalidation | Not yet | Invalidation is store-wide                                                   |
| Multi-instance coherence              | Not yet | No pub/sub or shared store                                                   |

## Findings

1. **Cache invalidation is store-wide.** A product or page edit clears every
   cached route for the store; this is correct but reduces hit rate.
2. **The cache is intentionally conservative.** Cookie-bearing requests are not
   served cached HTML, so personalized storefronts still render dynamically.
3. **No stale-while-revalidate.** The TTL expires and the next request pays the
   full render cost. A background refresh would require cloning the trusted
   request context and a second gateway call.
4. **No cache headers on the framework templates.** TanStack/Astro routes must
   opt in explicitly; otherwise the gateway adds `no-store`.
5. **Memory is bounded by entry size/count.** Eviction clears the whole cache
   when the entry cap is reached; a proper LRU is future work.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 14.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents opt-in caching and invalidation behavior.

## Gate decision

Phase 15 (generic Fetch Handler and additional framework support) may start.
SWR, cache tags and multi-instance coherence remain productization work.
