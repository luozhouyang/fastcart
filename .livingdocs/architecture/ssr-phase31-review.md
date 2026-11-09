---
type: Architecture Review
title: Universal SSR Runtime Phase 31 Review
description: Preview asset authorization cache review.
kind: review
timestamp: 2026-11-10T08:00:00Z
---

# Universal SSR Runtime Phase 31 Review

## Scope

Phase 31 reduces the per-asset revocation lookup:

- `AppContext` holds a bounded Moka cache of positive authorization checks.
- The key is `store_id:jti`; values are unit markers.
- A cache hit skips the store database lookup after signature, expiry and store
  binding checks.
- A cache miss performs the existing revocation lookup; only a non-revoked
  result is cached.
- The admin revoke handler invalidates the exact key after committing.
- Cache TTL is 30 seconds and capacity is 10,000 entries.

## Acceptance checklist

| Criterion                                        | Status         | Evidence                                  |
| ------------------------------------------------ | -------------- | ----------------------------------------- |
| Positive result is cached                        | Pass           | Asset route inserts on non-revoked lookup |
| Cache is bounded                                 | Pass           | 10k capacity, 30s TTL                     |
| Admin revocation invalidates the key             | Pass           | Handler invalidates after commit          |
| Revoked result is not cached                     | Pass           | Only `Ok(false)` inserts                  |
| Signature/store/expiry checks remain per request | Pass           | They run before the cache                 |
| Cross-instance invalidation                      | Not guaranteed | Other instances can be stale up to 30s    |
| Cache metrics                                    | Not added      | No hit/miss counters                      |
| Negative-cache abuse                             | Not applicable | Denials are not cached                    |

## Findings

1. **The cache covers only positive revocation checks.** Every request still
   verifies the HMAC, expiry and store binding, so a forged or cross-store token
   cannot use a cache entry.
2. **Revocation is immediate in the same process.** The admin endpoint
   invalidates after the transaction commits. A direct database write can leave a
   stale positive entry until TTL expiry; another FastCart process on the same
   data directory is rejected by the Phase 32 startup lock.
3. **The key is not secret.** `store_id:jti` appears in memory only; the token
   value is not used as a cache key.
4. **No cache metrics.** Hit/miss and size counters would make the effect visible
   but are not required for correctness.
5. **Moka's TTL is per entry.** A cached entry created just before revocation can
   survive up to 30 seconds if invalidation is missed.
6. **Capacity eviction is safe.** Eviction only causes another database lookup.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 31.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the positive cache and its staleness bound.

## Gate decision

The preview asset revocation check is cached with explicit same-process
invalidation. Multi-process operation is unsupported and enforced by the Phase 32
startup lock; cache metrics remain open.
