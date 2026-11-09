---
type: Architecture Review
title: Universal SSR Runtime Phase 9 Review
description: Activation, replacement, rollback and draining review.
kind: review
timestamp: 2026-11-09T10:00:00Z
---

# Universal SSR Runtime Phase 9 Review

## Scope

Phase 9 adds lifecycle operations to the runtime registry:

- `POST /internal/ssr/v1/deployments/{store_id}/activate` starts a replacement
  artifact, waits for `/__fastcart/health`, swaps the active pointer, records the
  previous deployment, and drains the old group after `ssr.drain_timeout_ms`.
- `POST /internal/ssr/v1/deployments/{store_id}/rollback` restarts the previous
  artifact and swaps back.
- A failed activation leaves the active deployment untouched.
- Asset serving falls back to retired asset roots so in-flight documents can
  still load their hashed files during the drain window.
- `drain_timeout_ms` is configurable and validated.

## Acceptance checklist

| Criterion                                           | Status  | Evidence                                                                                                         |
| --------------------------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------- |
| New deployment starts before old stops              | Pass    | Activation starts a new group and only then swaps the registry entry                                             |
| Health gates the swap                               | Pass    | Start fails closed when the group is not healthy                                                                 |
| Failed activation preserves the active deployment   | Pass    | Missing-artifact test still renders the previous deployment                                                      |
| Rollback works                                      | Pass    | Activation then rollback restores the previous deployment id                                                     |
| Old processes drain instead of stopping immediately | Pass    | Background stop after the configured delay                                                                       |
| Old assets remain reachable during draining         | Pass    | Retired asset-root fallback test                                                                                 |
| Lifecycle API is authenticated                      | Pass    | Internal bearer token required; wrong/missing token returns 403                                                  |
| Unknown store is rejected                           | Pass    | API resolves an active store before activation                                                                   |
| Registry survives restart                           | Not yet | Activation is in memory; restart reverts to `[[ssr.deployments]]`                                                |
| Admin/CLI UI                                        | Not yet | Endpoint is server-to-server only                                                                                |
| Concurrent activations are serialized               | Partial | Runtime starts are serialized by the manager lock; history ordering under concurrent requests is not load-tested |
| Artifact path confinement                           | Not yet | The trusted internal caller supplies an absolute path; a root allowlist is Phase 10                              |
| Credential rotation                                 | Not yet | The same static token is used for activation and commerce                                                        |

## Findings

1. **Activation is not persisted.** A restart returns to the boot-time
   `[[ssr.deployments]]` set. Phase 10 should write an atomic state file and
   reconcile it with config.
2. **The artifact path is trusted.** The endpoint is internal-token protected,
   but an allowlisted artifact root is still required before exposing this to
   less-trusted automation.
3. **Drained asset roots are never garbage-collected.** This is safe for
   correctness but grows disk usage; Phase 12 adds retention/GC.
4. **Rollback starts a fresh process.** It does not reuse a draining process,
   which keeps behavior simple but can briefly double capacity.
5. **No drain readiness probe.** The old group is stopped on a timer, not after
   in-flight request counts reach zero.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 9.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the activation and rollback endpoints.

## Gate decision

Phase 10 (security, persistence and observability) may start. Activation is
operational but not restart-persistent and not artifact-root-confined yet.
