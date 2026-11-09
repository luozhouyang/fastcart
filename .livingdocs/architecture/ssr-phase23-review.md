---
type: Architecture Review
title: Universal SSR Runtime Phase 23 Review
description: Lifecycle audit event review.
kind: review
timestamp: 2026-11-10T00:00:00Z
---

# Universal SSR Runtime Phase 23 Review

## Scope

Phase 23 records lifecycle actions in the platform audit log:

- `ssr.deployment.activate`
- `ssr.deployment.rollback`
- `ssr.preview.activate`
- `ssr.preview.deactivate`
- `ssr.credential.rotate`
- `ssr.credential.revoke`

Each entry carries the store, resource type/id, and non-secret metadata (artifact
path, preview flag, expiry, grace). Token values are never included.

## Acceptance checklist

| Criterion                                  | Status  | Evidence                                                 |
| ------------------------------------------ | ------- | -------------------------------------------------------- |
| Activation is audited                      | Pass    | Lifecycle test finds `ssr.deployment.activate`           |
| Rollback is audited                        | Pass    | Lifecycle test finds `ssr.deployment.rollback`           |
| Preview lifecycle is audited               | Pass    | Both preview actions present                             |
| Credential rotation/revocation are audited | Pass    | Both actions present; no token value in payload          |
| Audit failure does not break the operation | Pass    | Errors are logged and the response is unchanged          |
| Actor attribution                          | Partial | Internal caller has no user actor; `actor_id` is null    |
| Client IP                                  | Not yet | Internal endpoint does not propagate a trusted client IP |
| Immutable/tamper-evident audit             | Not yet | Existing platform audit table only                       |

## Findings

1. **Internal lifecycle actions have no user actor.** The audit entries are
   store-scoped but actorless; a future internal service identity should be
   recorded.
2. **Audit writes are best-effort.** A database failure does not fail activation
   or rotation; the failure is logged. A security-critical deployment may prefer
   fail-closed.
3. **Token material is excluded.** Rotation/revocation payloads contain only
   flags and timings, never the bearer value.
4. **Artifact paths are recorded.** These are operator-relevant but may reveal
   filesystem layout; audit access must be restricted.
5. **No IP address.** The trusted internal surface does not currently attach
   `ClientIp`, so `ip_address` remains null.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 23.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README notes lifecycle audit records.

## Gate decision

Lifecycle audit coverage is implemented. Service identity, IP attribution and
tamper-evident storage remain open.
