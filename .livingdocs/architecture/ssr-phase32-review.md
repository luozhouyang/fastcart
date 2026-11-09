---
type: Architecture Review
title: Universal SSR Runtime Phase 32 Review
description: Single-process startup lock review.
kind: review
timestamp: 2026-11-10T09:00:00Z
---

# Universal SSR Runtime Phase 32 Review

## Scope

Phase 32 enforces the supported single-node deployment model at startup:

- `AppState::initialize` acquires an advisory `fs2` lock next to the platform
  SQLite database before opening it.
- When SSR is enabled, it also locks
  `<ssr.runtime_dir>/fastcart-runtime.lock`.
- `try_lock_exclusive` fails startup if another process holds either lock.
- `:memory:` platform databases skip the platform lock for tests/embedding.
- Locks are held in `AppState` until process shutdown.

This replaces cross-process cache-invalidation work as a near-term goal. The
supported topology remains one `fastcart-serve` per data directory.

## Acceptance checklist

| Criterion                        | Status         | Evidence                                          |
| -------------------------------- | -------------- | ------------------------------------------------- |
| Second process fails fast        | Pass           | Unit test holds the lock twice                    |
| Lock is released on drop         | Pass           | Test re-acquires after dropping the first         |
| Runtime directory is protected   | Pass           | SSR-enabled initialization acquires a second lock |
| In-memory platform skips locking | Pass           | Unit test                                         |
| Lock files are not secrets       | Pass           | Advisory files contain no credentials             |
| Cross-process cache invalidation | Not needed     | Multi-process is now rejected                     |
| Network filesystem locking       | Not guaranteed | Depends on filesystem advisory-lock semantics     |
| PID/diagnostic content in lock   | Not added      | Error names the lock path only                    |

## Findings

1. **Locks are advisory.** They prevent a second FastCart process that cooperates
   with the convention; they do not prevent arbitrary tools from opening SQLite.
2. **Two lock scopes are covered.** The platform lock protects the data root; the
   runtime lock protects artifacts/credentials when SSR is enabled.
3. **Startup ordering matters.** The platform lock is acquired before
   `PlatformStore::open`, so a losing process cannot run migrations or jobs.
4. **Locks are per data directory, not per store.** That matches the
   single-platform-database architecture.
5. **Network filesystems may not honor `flock`.** Do not place the data directory
   on NFS/SMB and expect this guard to be reliable.
6. **No cross-process cache invalidation is implemented.** It is intentionally
   out of scope until a shared-database/HA design exists.
7. **Graceful restart requires the old process to exit.** A supervisor that
   starts the new process before the old one releases the lock will fail the new
   process rather than risk concurrent writers.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib process_lock_tests
```

## Documentation sync

- [ADR 0001](../adr/0001-single-node-modular-monolith-with-sqlite.md) records the
  single-node implementation status.
- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 32.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the lock and unsupported multi-process topologies.

## Gate decision

Single-process deployment is enforced at startup. HA/shared-database
coordination remains a separate architectural change.
