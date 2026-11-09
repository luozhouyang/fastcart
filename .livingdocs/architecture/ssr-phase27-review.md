---
type: Architecture Review
title: Universal SSR Runtime Phase 27 Review
description: Startup orphan artifact sweep review.
kind: review
timestamp: 2026-11-10T04:00:00Z
---

# Universal SSR Runtime Phase 27 Review

## Scope

Phase 27 sweeps artifact directories left by a previous process:

- After all configured/persisted deployments start successfully, initialization
  collects their digest directories into an active set.
- It scans `<runtime_dir>/artifacts` and removes directories not in that set.
- Active artifacts, including shared digest directories, are preserved.
- The sweep runs before the runtime is returned to the server.
- Scan/removal failures are logged; they do not prevent startup.
- The sweep is skipped when the runtime is disabled because initialization
  returns before scanning.

## Acceptance checklist

| Criterion                          | Status         | Evidence                                                              |
| ---------------------------------- | -------------- | --------------------------------------------------------------------- |
| Orphan directory is removed        | Pass           | Initialize test creates `artifacts/orphan-digest` and asserts removal |
| Active artifact survives           | Pass           | Test asserts the default deployment directory exists                  |
| Shared artifacts are preserved     | Pass           | Active set is built from all deployment/preview/default dirs          |
| Startup failure is not masked      | Pass           | Sweep runs only after every deployment starts                         |
| Runtime-disabled path is untouched | Pass           | Early return before artifact import                                   |
| Removal failure is non-fatal       | Pass           | Errors are logged                                                     |
| Sweep metric                       | Not yet        | Only an info log with the removed count                               |
| Concurrent-process safety          | Not guaranteed | Each process assumes one writer per runtime directory                 |

## Findings

1. **The active set is process-local.** Every deployment is imported and started
   before the set is built, so persisted deployments are included.
2. **Preview artifacts from the previous process are intentionally swept.** They
   cannot be restored by initialization and are no longer addressable.
3. **A shared runtime directory is unsupported.** Two FastCart processes pointing
   at the same `runtime_dir` could sweep each other's in-progress staging or
   active directories. Keep one writer per runtime directory.
4. **Symlinks are skipped.** The sweep only removes entries whose file type is a
   directory; this avoids following links out of the artifact root.
5. **No dedicated metrics.** Operators see an info log only; a sweep counter and
   error counter are reasonable follow-ups.
6. **Sweep failures retain data.** The process continues with orphaned disk usage
   rather than failing startup.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 27.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the startup scan.

## Gate decision

Startup orphan sweeping is implemented for one runtime writer. Concurrency
guards and sweep metrics remain open.
