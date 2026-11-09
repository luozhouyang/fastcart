---
type: Architecture Review
title: Universal SSR Runtime Phase 16 Review
description: SvelteKit adapter and cache service review.
kind: review
timestamp: 2026-11-09T17:00:00Z
---

# Universal SSR Runtime Phase 16 Review

## Scope

Phase 16 adds a SvelteKit template:

- `templates/storefront-sveltekit` uses `@sveltejs/adapter-cloudflare`.
- The adapter output is bundled with `wrangler deploy --dry-run` so the artifact
  contains a single Worker entry with no bare module specifiers.
- Client assets are packaged under `client/_app`; Rust exposes a compatibility
  `/_app/{*path}` route in addition to `/_fastcart/assets/*`.
- The generated workerd config now provides a no-op `cache` service and
  `cacheApiOutbound`, because SvelteKit's worktop cache calls the Cache API.
- SvelteKit's CSRF origin check is disabled for the trusted gateway; the
  gateway's origin checks and API CSRF remain the security boundary.

## Acceptance checklist

| Criterion                            | Status  | Evidence                                                                                |
| ------------------------------------ | ------- | --------------------------------------------------------------------------------------- |
| SvelteKit builds a FastCart artifact | Pass    | Bundled Worker + `_app` assets + manifest                                               |
| Artifact runs under workerd          | Pass    | Smoke test passes SSR, headers, streaming, cookies, redirects and errors                |
| No bare module imports               | Pass    | Wrangler bundles `_server`/`@sveltejs/kit` into one entry                               |
| Cache API does not crash             | Pass    | No-op cache service is wired via `cacheApiOutbound`                                     |
| SvelteKit assets resolve             | Partial | Rust `/_app/*` compatibility route; production asset prefix remains `/_fastcart/assets` |
| Browser hydration                    | Not yet | No browser harness                                                                      |
| Real Cache API                       | Not yet | Cache service returns 404; Rust is the actual HTTP cache                                |
| Adapter version contract             | Partial | SvelteKit 2.x / adapter-cloudflare 7.x                                                  |

## Findings

1. **Wrangler is used as the bundler.** `wrangler deploy --dry-run --outdir`
   produces a deployable single Worker without uploading. This keeps the
   artifact free of bare module imports.
2. **SvelteKit cannot express `/_fastcart/assets` as a relative asset path.**
   `kit.paths.assets` requires an absolute URL, so the template uses SvelteKit's
   default `/_app` and Rust provides a compatibility alias.
3. **The workerd cache service is a no-op.** It satisfies the adapter's Cache
   API calls without adding a second cache. Route-level caching remains the Rust
   gateway's responsibility.
4. **CSRF origin checking is disabled at the framework layer.** The trusted
   gateway strips/sets forwarding headers; public API CSRF is unchanged. This is
   acceptable only behind the FastCart gateway.
5. **SvelteKit asset URLs are hashed under `_app/immutable`.** They are
   compatible with the immutable cache policy, but the alias must remain
   available for the framework's document URLs.

## Verification

```bash
CI=true pnpm --filter @fastcart/storefront-sveltekit build:artifact
CI=true pnpm --filter @fastcart/storefront-sveltekit test:artifact
```

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 16.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- README lists the SvelteKit template.

## Gate decision

Phase 17 (advanced isolation or an alternative execution backend) may start.
The framework matrix now covers TanStack Start, Astro, SvelteKit and a generic
Fetch handler.
