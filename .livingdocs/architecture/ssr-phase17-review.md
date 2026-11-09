---
type: Architecture Review
title: Universal SSR Runtime Phase 17 Review
description: Scoped credentials and SSR observability review.
kind: review
timestamp: 2026-11-09T18:00:00Z
---

# Universal SSR Runtime Phase 17 Review

## Scope

Phase 17 completes the security/observability baseline:

- `[[ssr.deployments]]` accepts an optional `internal_token`; the gateway injects
  that deployment's token instead of the global one.
- When every deployment has a scoped token, the internal commerce API mounts only
  those credentials and rejects a token used outside its store.
- The auth middleware compares token candidates in constant time and attaches an
  `InternalSsrScope`; store middleware and lifecycle handlers enforce it.
- `Metrics` now exposes SSR requests, errors, cache hits/misses and active
  deployments in Prometheus text format.
- The server creates one metrics collector and shares it between `AppContext`
  and `UniversalSsr`.

## Acceptance checklist

| Criterion                         | Status  | Evidence                                                         |
| --------------------------------- | ------- | ---------------------------------------------------------------- |
| Per-deployment credentials        | Pass    | Config field, gateway injection, scoped token list               |
| Cross-store rejection             | Pass    | API test with a token scoped to another store returns 403        |
| Constant-time token comparison    | Pass    | Same `subtle::ct_eq` path for each candidate                     |
| Global fallback compatibility     | Pass    | Unscoped token used when deployments lack scoped tokens          |
| Lifecycle handlers enforce scope  | Pass    | `InternalSsrScope` checked before activation/rollback/preview    |
| SSR metrics are exported          | Pass    | Metrics render test asserts all five series                      |
| Deployment gauge tracks lifecycle | Pass    | Initialization/activation/deactivation/shutdown update the gauge |
| Token rotation                    | Not yet | Static config; no expiry or rotation endpoint                    |
| Per-store data policy             | Not yet | Preview still receives the same commerce scope                   |
| Metrics cardinality               | Pass    | No store/path labels; process-wide counters                      |

## Findings

1. **Scoped credentials are all-or-nothing.** If one deployment omits a token,
   the global token is used unscoped for compatibility. Production multi-store
   configs should set every deployment token.
2. **Runtime activation still uses the global token.** The lifecycle request does
   not carry a replacement deployment credential; a future activation body
   should accept a token reference.
3. **Preview shares the deployment token.** Preview Workers can reach the same
   commerce scope as production; read-only preview credentials are a future
   policy feature.
4. **Metrics are process-wide and label-free.** This avoids cardinality attacks
   but cannot break down SSR traffic by store; a bounded store label can be
   added after a cardinality review.
5. **No metric endpoint auth change.** `/metrics` remains on the existing
   health/metrics surface; deployments should restrict it at the proxy.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-http --lib metrics
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr internal_ssr_namespace
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 17.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document scoped tokens and SSR metrics.

## Gate decision

The requested security/observability scope is implemented. Remaining work is
productization: token rotation, preview data isolation, advanced isolation and
the deferred full test suite.
