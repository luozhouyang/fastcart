---
type: Architecture Review
title: Universal SSR Runtime Phase 19 Review
description: Persistent internal credential store review.
kind: review
timestamp: 2026-11-09T20:00:00Z
---

# Universal SSR Runtime Phase 19 Review

## Scope

Phase 19 persists rotated internal credentials:

- `InternalSsrCredentials::load` reads a versioned JSON state file, drops expired
  tokens, merges new configured tokens, and writes the result atomically.
- `rotate` persists immediately; the rotation endpoint propagates persistence
  errors as `500`.
- The state file is written through a staging file plus rename, with Unix mode
  `0600`.
- `ssr.credentials_path` selects the file; the default is
  `<runtime_dir>/internal-ssr-credentials.json`.
- The file is loaded for both auth and gateway token selection.

## Acceptance checklist

| Criterion                   | Status  | Evidence                                                                  |
| --------------------------- | ------- | ------------------------------------------------------------------------- |
| Rotations survive restart   | Pass    | Reload test validates the rotated token and rejects the expired old token |
| Atomic writes               | Pass    | Staging file + rename                                                     |
| Restrictive file mode       | Pass    | `0600` assertion on Unix                                                  |
| Expired tokens are filtered | Pass    | Load test with an expired configured token yields an empty set            |
| Config tokens remain usable | Pass    | Load merges new config tokens by token value                              |
| Rotation endpoint persists  | Pass    | `rotate` returns persistence errors to the API layer                      |
| External secret manager     | Not yet | Local file only                                                           |
| Encryption at rest          | Not yet | File mode is the only protection                                          |
| Revocation endpoint         | Not yet | Rotation expires old scoped tokens; no explicit revoke                    |
| Audit events                | Not yet | No rotation audit record                                                  |
| Multi-instance coherence    | Not yet | Per-process file; shared filesystem semantics are not coordinated         |

## Findings

1. **The file contains plaintext bearer secrets.** `0600` plus an
   operator-owned runtime directory is the current boundary; a secret manager or
   envelope encryption is the next hardening step.
2. **Persistence failure leaves the new token active in memory.** The endpoint
   returns `500`, signalling non-durability, but the process already accepts the
   token. A transactional rollback or restart-required flag would be stricter.
3. **Config merge never revokes.** Removing a token from config does not remove
   it from the persisted set; explicit revocation is required.
4. **Multi-instance writes are last-writer-wins.** Two servers sharing the file
   can overwrite each other; a single writer or database-backed store is needed
   for horizontal scaling.
5. **No file lock.** Concurrent rotation within one process is serialized by the
   credential `RwLock`; cross-process rotation is not.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --lib credential_tests
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 19.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document the credential file.

## Gate decision

Persistent rotation is implemented for a single-writer deployment. External
secret management, revocation and preview data isolation remain hardening work.
