---
type: Architecture Review
title: Universal SSR Runtime Phase 10 Review
description: Security, persistence and observability review.
kind: review
timestamp: 2026-11-09T11:00:00Z
---

# Universal SSR Runtime Phase 10 Review

## Scope

Phase 10 hardens the lifecycle boundary:

- Activation paths are canonicalized and, when `ssr.artifact_root` is set, must
  remain under that root.
- Successful activation/rollback writes
  `<runtime_dir>/deployment-state.json` atomically (`write` + `rename`).
- In default single-artifact mode, that state is loaded on restart and replaces
  the default deployment for the persisted stores.
- Render, activation and drain paths emit structured tracing fields (store,
  deployment, status, elapsed time).
- The runtime state file and artifact root stay inside the configured runtime
  directory.

## Acceptance checklist

| Criterion                     | Status  | Evidence                                                                                          |
| ----------------------------- | ------- | ------------------------------------------------------------------------------------------------- |
| Activation paths are confined | Pass    | Canonical root check; outside-root activation fails while the previous deployment keeps serving   |
| Activation survives restart   | Partial | Persisted state test passes in default mode; explicit `[[ssr.deployments]]` remains authoritative |
| State writes are atomic       | Pass    | Temporary file plus rename; failures are logged, not silently ignored                             |
| Invalid state fails safe      | Pass    | Unparseable state is logged and ignored, falling back to config                                   |
| No secrets in logs            | Pass    | Activation logs path and ids, never the internal token                                            |
| Request observability         | Pass    | Structured render logs with store, deployment, status and latency                                 |
| Runtime metrics               | Not yet | No Prometheus series for SSR requests/restarts                                                    |
| Per-store credentials         | Not yet | One static internal token still covers every store                                                |
| State file permissions        | Not yet | Runtime directory permissions are the only boundary                                               |
| Activation rate limiting      | Not yet | Caller is the trusted internal surface                                                            |

## Findings

1. **Explicit deployments win over persisted state.** Operators editing
   `[[ssr.deployments]]` get config-driven startup, so a runtime activation for
   one of those stores is lost after restart. A future reconciliation should
   compare a state revision with config.
2. **The state file is trusted input.** It lives under `runtime_dir`; anyone who
   can write it can point activation at another artifact. The runtime directory
   must be operator-owned and not writable by the Worker.
3. **No SSR metrics.** Tracing is useful for debugging but not for dashboards;
   Phase 12 should add low-cardinality counters/histograms.
4. **Credential rotation is still static.** The gateway and internal commerce
   API share one token; per-deployment tokens and rotation remain open.
5. **The activation endpoint is still a trusted-admin primitive.** There is no
   separate signed deployment manifest or approval workflow.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib config
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 10.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README and `config.example.toml` document `ssr.artifact_root` and persisted
  default-mode activation.

## Gate decision

Phase 11 (Astro integration) may start. Persistence and confinement cover the
trusted single-host path; credential scoping and metrics stay on the hardening
backlog.
