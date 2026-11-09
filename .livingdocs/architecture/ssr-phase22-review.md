---
type: Architecture Review
title: Universal SSR Runtime Phase 22 Review
description: Explicit internal credential revocation review.
kind: review
timestamp: 2026-11-09T23:00:00Z
---

# Universal SSR Runtime Phase 22 Review

## Scope

Phase 22 adds explicit revocation:

- `POST /internal/ssr/v1/deployments/{store_id}/revoke-token` accepts
  `{"token":"..."}`, requires a non-preview store credential, and removes that
  scoped token immediately.
- `InternalSsrCredentials::revoke` compares token values in constant time,
  removes only entries for the path store, persists the result, and returns 204
  even when the token was already absent.
- The revocation is effective for both auth and the next gateway token lookup.

## Acceptance checklist

| Criterion                          | Status  | Evidence                                             |
| ---------------------------------- | ------- | ---------------------------------------------------- |
| Token is rejected immediately      | Pass    | API test revokes the rotated token then gets 403     |
| Scope is enforced                  | Pass    | Uses the existing store-scoped auth middleware       |
| Preview tokens cannot revoke       | Pass    | Lifecycle `ensure_scope` rejects preview credentials |
| Persistence is updated             | Pass    | `revoke` calls the atomic credential persist         |
| Unknown token is idempotent        | Pass    | Returns 204 regardless of removal result             |
| Audit event                        | Not yet | Only tracing/API behavior; no audit record           |
| Revoking the current request token | Pass    | Request completes, subsequent requests fail          |
| Multi-instance coordination        | Not yet | Last writer wins on the shared file                  |

## Findings

1. **Revocation is immediate, unlike rotation.** Rotation keeps old tokens for a
   grace period; revocation intentionally drops the token at once. In-flight
   Worker calls using the revoked token fail immediately.
2. **Revocation is idempotent and non-enumerating.** The endpoint does not reveal
   whether the token existed.
3. **No audit record.** This is a security-sensitive action and should be added
   when the internal audit channel is formalized.
4. **The caller must know the token value.** There is no token-id listing or
   fingerprint endpoint; operators manage values out of band.
5. **Persistence errors return 500 after in-memory removal.** The token is gone
   from the running process but may reappear on restart if the write failed.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --lib credential_tests
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 22.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the revoke endpoint.

## Gate decision

Explicit revocation is implemented. Audit events, token fingerprints and
multi-instance coordination remain open.
