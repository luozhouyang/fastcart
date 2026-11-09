---
type: Architecture Review
title: Universal SSR Runtime Phase 28 Review
description: Framework preview-specific asset bases review.
kind: review
timestamp: 2026-11-10T05:00:00Z
---

# Universal SSR Runtime Phase 28 Review

## Scope

Phase 28 makes the framework templates produce preview-specific client asset
URLs that match FastCart's `/_fastcart/preview-assets/*` route:

- TanStack Start and Astro accept `FASTCART_ASSET_PREFIX` at build time.
- SvelteKit uses `kit.appDir = "_fastcart/preview-assets"` for preview builds
  because `paths.assets` rejects path-only values.
- Each template has `build:artifact:preview` and `test:artifact:preview` Node
  wrappers that set the correct environment for the existing build/smoke flow.
- Smoke tests accept `FASTCART_EXPECT_ASSET_PREFIX` and assert the home document
  references it.
- CI runs production and preview builds/smokes for all three templates.
- Astro emits an external stylesheet so the preview smoke can observe the asset
  prefix instead of relying only on inline HTML.

## Acceptance checklist

| Criterion                                     | Status        | Evidence                                              |
| --------------------------------------------- | ------------- | ----------------------------------------------------- |
| TanStack preview build uses the preview base  | Pass          | Preview build + smoke passed                          |
| Astro preview build uses the preview base     | Pass          | External stylesheet carries the prefix; smoke passed  |
| SvelteKit preview URLs use the preview prefix | Pass          | `appDir` build, packaged client, preview smoke passed |
| Production builds remain unchanged            | Pass          | All three production builds and smokes passed         |
| Common build/smoke scripts                    | Pass          | `build:artifact:preview`, `test:artifact:preview`     |
| CI covers preview artifacts                   | Pass          | Steps added to each template job                      |
| Single artifact usable for prod and preview   | Not supported | Separate build with different base/appDir             |
| Preview asset access control                  | Not added     | Hashed preview URLs remain public like production     |
| Runtime HTML rewrite                          | Not needed    | Build-time prefix                                     |

## Findings

1. **Preview bases are build-time.** A production artifact and a preview artifact
   are distinct builds; FastCart does not rewrite emitted asset URLs.
2. **SvelteKit has a framework constraint.** `kit.paths.assets` only accepts an
   absolute URL, so the preview build changes `appDir` to the path segment and
   `scripts/package.mjs` copies `_app/<appDir>` to `client/<appDir>`.
3. **The SvelteKit package layout is intentional.** The preview artifact's client
   root contains `_fastcart/preview-assets/...`; FastCart's existing preview route
   maps the URL path directly to the client root.
4. **Astro needs an external stylesheet.** The original template had no emitted
   client asset, so the smoke could not observe the prefix. `inlineStylesheets:
"never"` plus a small stylesheet makes the assertion meaningful.
5. **CI doubles template builds.** This is accepted for correctness; a future
   matrix split can run production and preview jobs in parallel.
6. **Preview assets are public hashed files.** Possession of the URL is the
   access model, same as production; the preview HTML remains token-gated.

## Verification

```bash
pnpm --filter @fastcart/storefront-tanstack build:artifact:preview
pnpm --filter @fastcart/storefront-tanstack test:artifact:preview
pnpm --filter @fastcart/storefront-astro build:artifact:preview
pnpm --filter @fastcart/storefront-astro test:artifact:preview
pnpm --filter @fastcart/storefront-sveltekit build:artifact:preview
pnpm --filter @fastcart/storefront-sveltekit test:artifact:preview
```

Production builds and smokes were also rerun for all three templates.

## Documentation sync

- [ADR 0007](../adr/0007-universal-ssr-runtime-with-workerd.md) records Phase 28.
- [Issue 0003](../issues/0003-implement-universal-ssr-runtime-with-workerd.md)
  records the same status.
- Root and template READMEs document the preview builds.

## Gate decision

All three framework templates can build preview-prefixed artifacts. A single
dual-use artifact and preview asset authorization remain open.
