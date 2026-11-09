---
type: Architecture Review
title: Universal SSR Runtime Phase 24 Review
description: Credential backend abstraction and file refresh review.
kind: review
timestamp: 2026-11-10T01:00:00Z
---

# Universal SSR Runtime Phase 24 Review

## Scope

Phase 24 extracts credential storage into a pluggable crate:

- New `fastcart-ssr-credentials` crate with `CredentialBackend`,
  `CredentialManager`, `CredentialMode` and the file backend.
- File backend keeps the v1 JSON format, atomic rename, `0600` mode, content-hash
  revision and an advisory file lock for compare-and-swap writes.
- `CredentialManager` persists before swapping the live snapshot; failures leave
  the old snapshot active.
- `[ssr.credentials]` config adds `backend`, `mode`, `path`,
  `refresh_interval_seconds`, `required` and `seed_from_config`.
- A background refresh loop reloads the file when its content revision changes.
- `read_only` rejects rotation/revocation with `409`.
- Per the agreed scope, no Kubernetes or Vault KV v2 backend is included.

## Acceptance checklist

| Criterion                                   | Status          | Evidence                                                |
| ------------------------------------------- | --------------- | ------------------------------------------------------- |
| Backend abstraction exists                  | Pass            | Trait + file implementation in a separate crate         |
| Existing credential format remains readable | Pass            | Same `version`/`tokens` JSON fields                     |
| Atomic writes and `0600` mode               | Pass            | File backend test                                       |
| CAS conflicts are detected                  | Pass            | Revision test returns `Conflict`                        |
| Rotate persists before swap                 | Pass            | Manager test rotates then reloads from a new manager    |
| Read-only mode rejects writes               | Pass            | Manager test returns `ReadOnly`                         |
| External file changes refresh               | Pass            | Background refresh loop; revision comparison            |
| Startup policy                              | Pass            | `required` fails startup; otherwise config fallback     |
| Seed semantics                              | Pass            | Empty backend seeded once; existing state authoritative |
| Env/Kubernetes/Vault backend                | Not implemented | Explicitly out of scope this phase                      |
| Credential backend metrics                  | Not yet         | Refresh failures only logged                            |
| Multi-instance file CAS                     | Partial         | Advisory lock; shared-filesystem semantics vary         |
| Secret encryption at rest                   | Not yet         | File mode only                                          |

## Findings

1. **Config is no longer merged every startup.** Once the backend has state, it
   is authoritative. This prevents revoked config tokens from reappearing but
   means adding a token by config requires deleting/updating the backend file or
   using the rotation API.
2. **Refresh is revision-based polling.** The file backend hashes contents; a
   changed file replaces the in-memory set. The loop skips missed ticks and
   logs failures without failing requests.
3. **Rotation is persist-first.** A backend conflict or write failure returns an
   error and leaves the previous snapshot active, unlike the prior in-memory
   first behavior.
4. **No external backend yet.** The trait is ready, but the only implementation is
   the local file. Kubernetes/Vault adapters were intentionally excluded.
5. **The read-only mode still uses configured tokens when the backend is empty.**
   Use `seed_from_config = false` to make an external backend truly
   authoritative.
6. **Backend metrics are missing.** Failures surface through logs and API errors
   only; the next phase should add low-cardinality counters.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr-credentials
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 24.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document `[ssr.credentials]`.

## Gate decision

The backend abstraction and file refresh path are implemented. External secret
backends, metrics and encryption remain follow-up work.
