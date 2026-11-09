---
type: Architecture Review
title: Universal SSR Runtime Phase 6 Review
description: Storefront SDK and server-only commerce API review.
kind: review
timestamp: 2026-11-09T07:00:00Z
---

# Universal SSR Runtime Phase 6 Review

## Scope

Phase 6 adds the server-to-server commerce boundary and its client:

- `GET|POST /internal/ssr/v1/stores/{store_id}/...` mounts the existing
  storefront handlers behind a bearer-token check.
- Store selection comes from the path, not `Host`; only active stores resolve.
- `@fastcart/storefront-sdk/server` exposes
  `createServerStorefrontClient`, which rewrites `/api/v1/store/*` to the
  internal namespace and attaches `Authorization: Bearer <token>`.

The legacy `/internal/storefront` surface is unchanged and remains
secret-gated.

## Acceptance checklist

| Criterion                                                               | Status  | Evidence                                                                                                    |
| ----------------------------------------------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------- |
| Search, product, collection, cart and context access exists server-side | Pass    | Existing store handlers are mounted under the internal namespace                                            |
| Internal API is not browser-accessible by default                       | Pass    | Only mounted when `internal_ssr_token` is configured; bearer token required                                 |
| Store context comes from the server                                     | Pass    | Path middleware resolves the active store and inserts a trusted `StoreContext`; `Host` is ignored           |
| The browser cannot select a store                                       | Pass    | Internal middleware never calls the public host resolver                                                    |
| Cross-store requests are rejected                                       | Partial | The path store must be active; one token is still process-wide. Per-deployment scoping is Phase 8           |
| SDK consumes the internal API                                           | Partial | Server client and path/auth tests exist; the template does not call it until the binding/env contract lands |
| Customer and cart cookies propagate                                     | Pass    | `customer_auth_middleware` runs and the SDK sends credentials                                               |
| Errors are controlled                                                   | Pass    | Missing/wrong token returns 403; unknown store returns 404                                                  |
| Internal credentials are short-lived                                    | Not yet | Token is static configuration; rotation is Phase 9                                                          |
| API is documented                                                       | Partial | Router docs updated; OpenAPI coverage for the internal namespace is deferred                                |

## Findings

1. **The internal token is shared by the gateway and the commerce API.** The
   Worker receives it as `x-fastcart-internal-token`, so a compromised Worker
   can call the internal API for any active store. Phase 8 must bind tokens to a
   single deployment/store and shorten their lifetime.
2. **The template does not call the SDK yet.** The Worker has no
   `FASTCART_COMMERCE_ORIGIN` binding in its workerd config and no trusted
   request-scoped token source. Phase 8 adds the binding contract and a route
   that exercises the SDK through the Worker.
3. **Internal routes reuse public handlers.** This is intentional (one set of
   business rules), but internal-only fields must not be added to public DTOs
   without a separate serializer.
4. **No rate limit specific to the internal namespace.** Loopback/token
   protection is the current boundary; Phase 10 adds observability and abuse
   controls.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr
CI=true pnpm --filter @fastcart/storefront-sdk test
CI=true pnpm --filter @fastcart/storefront-sdk typecheck
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 6.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and API module docs list the internal namespace.

## Gate decision

Phase 7 (static assets and hybrid rendering) may start. Token scoping and SDK
execution inside the Worker move to Phase 8 with the multi-store registry.
