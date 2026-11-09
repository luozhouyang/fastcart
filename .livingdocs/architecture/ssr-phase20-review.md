---
type: Architecture Review
title: Universal SSR Runtime Phase 20 Review
description: Preview data isolation and read-only credentials review.
kind: review
timestamp: 2026-11-09T21:00:00Z
---

# Universal SSR Runtime Phase 20 Review

## Scope

Phase 20 isolates preview commerce access:

- Internal credentials carry a `preview` flag, persisted with the credential
  store.
- Production renders select production credentials; preview renders select only
  preview credentials. A preview request never receives the production token.
- A preview credential is read-only on store routes: unsafe methods return 403.
- Deployment lifecycle handlers reject preview credentials.
- Preview deployments without a preview credential receive no internal token.
- Rotation accepts `"preview": true` and scopes the grace window to the same
  preview/production class.

## Acceptance checklist

| Criterion                                  | Status  | Evidence                                                                              |
| ------------------------------------------ | ------- | ------------------------------------------------------------------------------------- |
| Preview does not receive production token  | Pass    | Preview gateway test observes `preview-token`, production observes `production-token` |
| Preview credentials are read-only          | Pass    | POST with preview token returns 403; GET succeeds                                     |
| Lifecycle operations reject preview tokens | Pass    | `ensure_scope` rejects preview scope                                                  |
| Missing preview credential denies access   | Pass    | Preview context has no token when no preview credential is configured                 |
| Credential flags persist                   | Pass    | `preview` is serialized in the credential state file                                  |
| Preview token rotation                     | Pass    | Rotation request supports `preview: true`                                             |
| Field-level draft data policy              | Not yet | Read-only is enforced; draft-only fields are not separated                            |
| Preview asset isolation                    | Not yet | Assets still resolve through the production asset roots                               |
| Preview token expiry UX                    | Partial | Expiry exists; no automatic deactivation of the preview deployment                    |

## Findings

1. **Read-only is enforced by HTTP method.** Preview can still read any store
   endpoint the production token can read, including customer/cart data if
   cookies are forwarded. A read-only scope is not a data minimization policy.
2. **Preview assets remain production assets.** The preview document may
   reference hashed assets that resolve from the production artifact root.
3. **No preview credential means no API access.** This is the safe default but
   makes preview Workers less capable unless explicitly configured.
4. **Lifecycle is production/admin-only.** Preview tokens cannot activate,
   rollback or rotate; production rotation can create preview credentials.
5. **Persisted flags are trusted.** The credential file is operator-owned; a
   compromised writer can mark a production token as preview or vice versa.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr internal_ssr_namespace
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --lib credential_tests
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 20.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents preview-only credentials and read-only behavior.

## Gate decision

Preview read isolation is implemented. Field-level draft data policies,
preview asset roots and automatic preview deactivation remain hardening work.
