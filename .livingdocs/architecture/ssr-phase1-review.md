---
type: Architecture Review
title: Universal SSR Runtime Phase 1 Review
description: Process supervisor and HTTP gateway acceptance review.
kind: review
timestamp: 2026-11-09T04:00:00Z
---

# Universal SSR Runtime Phase 1 Review

## Scope

Phase 1 of the [Universal SSR Runtime](universal-ssr-runtime.md) plan builds the
smallest Rust-to-workerd execution path:

- `crates/fastcart-ssr/src/process.rs` — spawn, readiness, health, SIGTERM and
  forced-kill shutdown, stdout/stderr capture.
- `crates/fastcart-ssr/src/gateway.rs` — loopback HTTP proxy with hop-by-hop
  filtering and a streamed response body.
- `crates/fastcart-ssr/src/config.rs` — typed process configuration and
  loopback mapping for wildcard binds.
- `crates/fastcart-ssr/tests/workerd_process.rs` — real-child integration test.

## Acceptance checklist

| Criterion                            | Status | Evidence                                                                           |
| ------------------------------------ | ------ | ---------------------------------------------------------------------------------- |
| Rust starts workerd                  | Pass   | Integration test starts the pinned binary with a generated Cap'n Proto file        |
| Rust detects process failure         | Pass   | `reports_an_early_process_exit` uses `/bin/false` and receives `EarlyExit`         |
| Rust forwards requests               | Pass   | POST `/echo?x=1` returns the expected method, path, query and body                 |
| Standard status and headers survive  | Pass   | `/created` returns 201 and `x-ssr-test: yes`                                       |
| Streaming response path is preserved | Pass   | `SsrGateway` returns `SsrBody::Stream` backed by `reqwest::bytes_stream`           |
| Graceful shutdown                    | Pass   | SIGTERM via `nix`; timeout then forced kill; second `stop()` is a no-op            |
| No orphaned process                  | Pass   | `kill_on_drop` plus `Drop`/`stop`; integration test checks `is_running()` is false |
| Health endpoint                      | Pass   | Configurable health path; fixture exposes `/__fastcart/health`                     |
| Logs captured                        | Pass   | Bounded stdout/stderr ring exposed by `recent_logs()`                              |

## Deliberate limits

1. **Not mounted in `fastcart-server`.** The gateway is a crate API, not an Axum
   route. Phase 4 owns the server wiring, trusted context injection and browser
   path. A Phase 1 crate-level proxy is intentionally not described as an
   end-to-end browser feature.
2. **Request bodies are buffered.** `forward` takes `Request<Bytes>`; response
   bodies stream. Streaming request bodies are a Phase 4 concern.
3. **Caller supplies Cap'n Proto.** Configuration generation is Phase 3. Phase
   1 validates that a supplied config is executable and supervised.
4. **Host is stripped.** The original storefront host and trusted FastCart
   headers must be injected by the Phase 4 gateway; forwarding the loopback
   `Host` would lose the storefront origin.
5. **No artifact/deployment model.** There is no artifact validation, registry,
   rollback or per-store routing yet; those are Phase 2/3/8.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-ssr
CARGO_HOME="$PWD/.cargo-home" cargo clippy -p fastcart-ssr --all-targets -- -D warnings
CARGO_HOME="$PWD/.cargo-home" cargo fmt --all -- --check
```

The integration test skips when neither `FASTCART_WORKERD_BIN` nor
`experiments/workerd/bin/workerd` is present, so CI does not download the
152 MB runtime.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 1
  as implemented and the overall implementation as partial.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status and the Phase 4 wiring dependency.
- `crates/fastcart-ssr/README.md` documents the crate boundary.

## Gate decision

Phase 2 (artifact contract and packager) may start. The process and gateway
contracts are stable enough for the artifact schema to target them; Phase 4 will
add the Axum route, trusted headers and browser-visible streaming.
