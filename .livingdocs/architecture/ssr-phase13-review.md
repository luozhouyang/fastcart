---
type: Architecture Review
title: Universal SSR Runtime Phase 13 Review
description: Preview deployment support review.
kind: review
timestamp: 2026-11-09T14:00:00Z
---

# Universal SSR Runtime Phase 13 Review

## Scope

Phase 13 adds preview deployments:

- `[[ssr.deployments]]` entries can set `preview = true`; a store may have one
  production and one preview deployment.
- Preview deployments are kept in a separate registry and never replace the
  production entry.
- `POST /internal/ssr/v1/deployments/{store_id}/preview/activate` and
  `/preview/deactivate` start/stop a preview deployment with the internal token.
- `/preview/{path}?token=...` verifies the existing signed preview token, then
  forwards to the preview deployment when one exists.
- The gateway strips the preview token from the URL, sets the store Host, and
  injects `x-fastcart-preview: 1`; client-supplied preview headers are stripped.
- Preview activation is persisted alongside production activation in default
  mode.

## Acceptance checklist

| Criterion                            | Status              | Evidence                                                                                  |
| ------------------------------------ | ------------------- | ----------------------------------------------------------------------------------------- |
| Preview does not replace production  | Pass                | Separate map and direct test: production deployment id is unchanged while preview renders |
| Preview activation/deactivation      | Pass                | Runtime test plus authenticated API routes                                                |
| Preview uses signed tokens           | Pass                | Existing `/preview` token verification is reused                                          |
| Preview flag reaches the Worker      | Pass                | `SsrRequestContext.preview` injects `x-fastcart-preview`; spoofed values are stripped     |
| Preview URL strips token             | Pass                | `preview_runtime_url` removes `token`/`preview_handle` before forwarding                  |
| Preview can be configured at startup | Pass                | `preview = true` in `[[ssr.deployments]]`                                                 |
| Preview state survives restart       | Pass (default mode) | State file records `preview` entries; loaded in default mode                              |
| Preview-specific assets              | Not yet             | Assets still resolve through the production asset root                                    |
| Draft page data injection            | Not yet             | The signed token's `page_id` is not forwarded to framework workers                        |
| Production data policy               | Partial             | Preview inherits production data; a separate policy is not enforced                       |
| Preview expiry                       | Not yet             | Explicit deactivation only                                                                |

## Findings

1. **Preview is deployment-level, not page-level.** The existing signed token
   authorizes the request but its `page_id` is not consumed by the universal
   path; the framework artifact must decide what to render.
2. **Preview assets are a gap.** A preview HTML document may reference assets
   from its artifact, but `/_fastcart/assets/*` has no preview selector. The
   current fallback serves the production artifact's client tree.
3. **Production data remains reachable.** Preview Workers receive the same
   internal token and commerce API as production. A future policy must mint a
   read-only preview credential or a scoped data endpoint.
4. **No automatic preview expiry.** The preview token expires, but the preview
   deployment stays active until deactivated or the process restarts.
5. **Preview state persistence follows default mode.** Explicit
   `[[ssr.deployments]]` remains authoritative on restart.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr preview_route
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr deployment_lifecycle
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 13.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents preview activation and the preview route behavior.

## Gate decision

Phase 14 (incremental static regeneration) may start. Preview isolation and
preview asset routing remain follow-up hardening.
