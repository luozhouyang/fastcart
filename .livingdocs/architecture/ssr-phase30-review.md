---
type: Architecture Review
title: Universal SSR Runtime Phase 30 Review
description: Asset-time preview token revocation review.
kind: review
timestamp: 2026-11-10T07:00:00Z
---

# Universal SSR Runtime Phase 30 Review

## Scope

Phase 30 makes preview token revocation apply to preview assets:

- After cookie signature and store binding checks, the asset route loads the
  Host-resolved store database.
- It calls the existing `preview_token_revocations` lookup for the token's `jti`.
- A revoked token returns `403`; a database failure returns `500`.
- The lookup uses the existing primary-key index on `jti`.
- The preview page render already performed the same check; assets now match it.

## Acceptance checklist

| Criterion                            | Status    | Evidence                                         |
| ------------------------------------ | --------- | ------------------------------------------------ |
| Revoked token is rejected for assets | Pass      | API test inserts a revocation row and gets 403   |
| Valid token still serves assets      | Pass      | Existing authenticated asset test                |
| Store binding remains enforced       | Pass      | Cross-store token still returns 403              |
| Revocation is per token (`jti`)      | Pass      | Existing signed payload identifier               |
| Database failure is fail-closed      | Pass      | Error path returns 500                           |
| No token material in logs            | Pass      | Reuses the verifier and DB lookup                |
| Authorization cache                  | Not used  | One primary-key lookup per preview asset request |
| Cross-origin preview support         | Not added | Cookie remains same-origin/path-scoped           |

## Findings

1. **Assets now require a database round trip on a cache miss.** Preview traffic
   is low and the `jti` lookup uses the primary key; Phase 31 adds a short
   positive cache with admin invalidation.
2. **Revocation is immediate.** A revoked preview token cannot fetch assets even
   if its cookie is still present, until the revocation row is pruned after
   expiry.
3. **Fail-closed on database errors.** A transient store database failure denies
   the asset rather than serving it.
4. **Cookie and token share identity.** The cookie carries the signed token, so
   revocation only needs `jti`, not a separate asset grant record.
5. **No authorization cache.** A future cache would need invalidation on
   revocation; the current direct lookup avoids that complexity.
6. **Expired revocation rows are harmless.** Admin maintenance and preview
   rendering already prune/ignore expired rows.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 30.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents asset-time revocation.

## Gate decision

Preview assets honor token revocation. Cross-origin preview hosting and
authorization caching remain open.
