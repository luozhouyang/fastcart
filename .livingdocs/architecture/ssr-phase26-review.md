---
type: Architecture Review
title: Universal SSR Runtime Phase 26 Review
description: Retired artifact reference counting and garbage collection review.
kind: review
timestamp: 2026-11-10T03:00:00Z
---

# Universal SSR Runtime Phase 26 Review

## Scope

Phase 26 reclaims digest-addressed artifacts after deployments are replaced or
previews are deactivated:

- Each `Deployment` records its imported `artifact_dir`.
- `UniversalSsr` holds a reference-count map keyed by artifact directory.
- Activation/preview insertion increments the count; drain completion decrements
  it.
- The last reference triggers removal of the digest directory and its asset roots.
- Shared artifacts are retained while any active deployment or preview uses them.
- Drain failures retain the artifact rather than deleting a live runtime.
- No new configuration; reuse `drain_timeout_ms`.

## Acceptance checklist

| Criterion                                  | Status  | Evidence                                                                                 |
| ------------------------------------------ | ------- | ---------------------------------------------------------------------------------------- |
| Deactivated preview artifact is removed    | Pass    | Runtime test captures the preview digest directory and asserts it disappears after drain |
| Shared artifact is retained                | Pass    | Reference-count design; test uses distinct preview artifact                              |
| Asset roots are removed with the directory | Pass    | `release_artifact` prunes both root lists                                                |
| Drain failure retains assets               | Pass    | Release runs only after `stop_group` succeeds                                            |
| Production replacement uses the same path  | Pass    | `replace` schedules the previous deployment for drain                                    |
| Startup orphan sweep                       | Not yet | Only refs created during this process are collected                                      |
| GC failure metric                          | Not yet | Failures are logged                                                                      |
| GC configuration/retention window          | Partial | Uses the drain timeout; no separate retention knob                                       |

## Findings

1. **Reference counting is digest-based.** Two deployments importing identical
   content share one digest directory; the last release deletes it.
2. **Deletion follows group stop.** A failed stop logs and retains the artifact,
   leaving reclaim for operator cleanup.
3. **In-flight requests are protected by the drain window.** After
   `drain_timeout_ms`, the group is stopped before the directory is removed.
4. **Startup orphans remain.** Artifact directories left by a crashed process are
   not scanned or deleted at startup. A future sweep should only remove digest
   directories absent from the active/persisted set.
5. **No dedicated metric.** The existing deployment gauge and logs are the only
   signals; a GC counter/error counter would be a useful follow-up.
6. **Root pruning uses path prefixes.** Both asset root lists are filtered by
   `starts_with(artifact_dir)`, which is safe for store-owned digest paths.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 26.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents retired-artifact cleanup.

## Gate decision

Retired artifacts are collected for this process. Startup orphan sweeping and GC
metrics remain follow-up work.
