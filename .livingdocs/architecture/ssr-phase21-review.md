---
type: Architecture Review
title: Universal SSR Runtime Phase 21 Review
description: Preview asset roots and prefix propagation review.
kind: review
timestamp: 2026-11-09T22:00:00Z
---

# Universal SSR Runtime Phase 21 Review

## Scope

Phase 21 separates preview assets from production:

- `GET /_fastcart/preview-assets/{*path}` resolves the store from `Host` and
  serves from the active preview artifact's client directory.
- The gateway injects a trusted `x-fastcart-assets-prefix` header:
  `/_fastcart/preview-assets` for preview requests and `/_fastcart/assets` for
  production.
- The generic Fetch Handler emits its module URL from that header.
- Retired preview asset roots remain readable after preview replacement.
- Preview asset requests do not fall back to production asset roots.

## Acceptance checklist

| Criterion                                    | Status  | Evidence                                                                  |
| -------------------------------------------- | ------- | ------------------------------------------------------------------------- |
| Preview assets resolve from preview artifact | Pass    | Runtime test returns preview `asset-b` while production returns `asset-a` |
| Production assets remain isolated            | Pass    | Production asset test unchanged                                           |
| Preview asset route requires a store         | Pass    | Host resolution; 404 without a preview deployment                         |
| Deactivated preview assets disappear         | Pass    | 404 after preview deactivation                                            |
| Prefix is trusted                            | Pass    | Gateway strips client `x-fastcart-*` and injects the prefix               |
| Generic template uses the prefix             | Pass    | Module URL is header-driven                                               |
| TanStack/Astro/SvelteKit preview builds      | Not yet | Their Vite bases are build-time; preview builds need a preview base       |
| Asset GC                                     | Not yet | Retired preview roots accumulate                                          |
| Range/ETag/304                               | Pass    | Shared asset handler                                                      |

## Findings

1. **Asset prefix is a request header, not an HTML rewrite.** Preview Workers
   must use `x-fastcart-assets-prefix` when emitting asset URLs. The generic
   template does; framework templates need preview-specific builds or a runtime
   asset-base hook.
2. **Preview assets are not production assets.** Falling back to production
   would hide missing preview files and could serve a mismatched hash, so the
   preview route returns 404 instead.
3. **Deactivation removes the active root but retains retired roots.** In-flight
   preview documents can still load assets during the drain window; GC is future
   work.
4. **Asset requests still require Host resolution.** A preview asset request
   without a resolvable Host is 404 even if a preview deployment exists.
5. **No preview token is required for assets.** Possession of a hashed asset URL
   is the access model, same as production. Preview HTML remains token-gated.

## Verification

```bash
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-server --lib universal
CARGO_HOME="$PWD/.cargo-home" cargo test -p fastcart-api --test storefront_ssr universal_assets
CI=true pnpm --filter @fastcart/storefront-generic test:artifact
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 21.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README documents the preview asset route and prefix header.

## Gate decision

Preview assets are isolated for header-aware Workers. Framework preview builds
and asset GC remain follow-up work.
