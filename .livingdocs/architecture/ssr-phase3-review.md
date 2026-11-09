---
type: Architecture Review
title: Universal SSR Runtime Phase 3 Review
description: Runtime manager, configuration generation and restart supervision review.
kind: review
timestamp: 2026-11-09T05:00:00Z
---

# Universal SSR Runtime Phase 3 Review

## Scope

Phase 3 turns the one-shot workerd launcher into a managed runtime group:

- `crates/fastcart-ssr/src/runtime/config_generator.rs` — generates
  `groups/<id>/config.capnp` from a validated artifact, including every server
  module and the entry module first.
- `crates/fastcart-ssr/src/runtime/manager.rs` — `RuntimeManager` with group
  start/stop/restart, health probing, backoff and restart limits.
- `crates/fastcart-ssr/tests/runtime_manager.rs` — real workerd lifecycle test.

## Acceptance checklist

| Criterion                                                | Status  | Evidence                                                                                             |
| -------------------------------------------------------- | ------- | ---------------------------------------------------------------------------------------------------- |
| Runtime can be started and stopped through a manager API | Pass    | `start_group`/`stop_group` integration test                                                          |
| Process exits are detected                               | Partial | `WorkerdProcess` detects early exit and health reports `Stopped`; no background exit monitor         |
| Health checks work                                       | Pass    | `/__fastcart/health` is probed; manager updates status                                               |
| Restart backoff works                                    | Pass    | `restart_group` sleeps the remaining backoff; test asserts a minimum elapsed time                    |
| Restart limit works                                      | Pass    | `max_restarts = 1`; the second restart returns `RestartLimit` and marks `Failed`                     |
| Invalid runtime configuration is rejected                | Pass    | `WorkerdConfig::validate` checks binary/config/timeouts; generator rejects unsafe paths and symlinks |
| Configuration generation is deterministic                | Pass    | Modules are globally sorted with the entry first; relative embed paths are stable                    |
| Runtime logs are captured                                | Partial | Bounded stdout/stderr ring in `WorkerdProcess`; no log files                                         |
| Runtime directory layout                                 | Partial | `groups/<id>/config.capnp` exists; logs/state directories are not created                            |

## Findings

1. **No background supervisor yet.** The manager can restart on request, but it
   does not watch for a crash and auto-restart. That belongs with the deployment
   lifecycle and in-flight draining in Phase 9.
2. **One process per group.** Phase 3 starts a new process for each group; it
   does not multiplex multiple services into one workerd config yet.
3. **Health path is a contract.** Generated configs use
   `/__fastcart/health`; framework templates must expose it. The Phase 0
   TanStack/Astro fixtures do not, so only the Phase 3 fixture can be managed
   until Phase 5/11.
4. **Generated configs include all JS modules.** Client JS is listed as a
   module but never imported by the Worker. This is harmless but should be
   narrowed when the packager emits a server-only module directory.
5. **No persistent group registry.** Restart counters and status live in memory;
   a manager restart resets them. Phase 8 adds the deployment registry.
6. **Config generator has no bindings.** Internal Commerce API bindings and
   trusted context arrive with the Phase 4 gateway.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr
CARGO_HOME="$PWD/.cargo-home" cargo clippy -p fastcart-ssr --all-targets -- -D warnings
CARGO_HOME="$PWD/.cargo-home" cargo fmt --all -- --check
```

The manager integration test skips when no pinned workerd binary is available.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 3
  as implemented and the overall implementation as partial.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- `crates/fastcart-ssr/README.md` documents the manager and generated layout.

## Gate decision

Phase 4 (SSR gateway and server integration) may start. The manager and
generated config are stable enough for the Axum route to own process lifetime
and for trusted context to be injected at the gateway boundary.
