---
type: Architecture Review
title: Universal SSR Runtime Phase 18 Review
description: Online internal token rotation review.
kind: review
timestamp: 2026-11-09T19:00:00Z
---

# Universal SSR Runtime Phase 18 Review

## Scope

Phase 18 adds online rotation for deployment-scoped internal credentials:

- `InternalSsrCredentials` is a shared, mutable token set used by both auth and
  the universal gateway.
- `POST /internal/ssr/v1/deployments/{store_id}/rotate-token` adds a new scoped
  token with optional `expires_at`, expires older scoped tokens after
  `grace_seconds` (default 300), and returns 204.
- Auth validates candidates in constant time, purges expired tokens, and
  attaches the store scope.
- The gateway selects the newest active token for the request's store through
  `SsrRequestContext.internal_token`; the static gateway token remains a
  fallback.

## Acceptance checklist

| Criterion                        | Status  | Evidence                                                     |
| -------------------------------- | ------- | ------------------------------------------------------------ |
| Rotation is authenticated        | Pass    | Existing internal bearer middleware; wrong scope returns 403 |
| New token is usable immediately  | Pass    | API test rotates then queries `/context` with the new token  |
| Old token has a grace window     | Pass    | Rotation sets older scoped expiry to now + grace             |
| Expired tokens are rejected      | Pass    | Auth purges expired entries; invalid expiry returns 400      |
| New token reaches the Worker     | Pass    | Runtime test rotates credentials and observes the new header |
| Rotation is store-scoped         | Pass    | Scope check runs before rotation                             |
| Rotation persists across restart | Not yet | In-memory; restart reverts to config                         |
| Global token rotation            | Not yet | Endpoint rotates per-store scoped entries only               |
| Audit trail                      | Not yet | No audit event for token rotation                            |
| Concurrent rotation safety       | Partial | Single `RwLock`; last writer wins                            |

## Findings

1. **Rotation is immediate for the gateway, grace-limited for auth.** A Worker
   that receives the new request token calls the API with it; older in-flight
   calls keep working until their grace expiry.
2. **Restart reverts rotation.** Startup rebuilds credentials from config. A
   persistent credential store or secret manager is the next step.
3. **The endpoint does not require a dedicated rotation permission.** It uses
   the store-scoped internal token; operators should restrict the internal
   surface at the network layer.
4. **No audit event.** Rotation is a security-relevant action and should emit an
   audit record once the internal audit channel exists.
5. **Expiry is unix seconds.** Callers must use the same clock assumptions as
   the server; a skew may reject a token early.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 18.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the rotation endpoint and grace behavior.

## Gate decision

Online rotation is implemented. Persistent credentials, audit events and
external secret-manager integration remain hardening work.
