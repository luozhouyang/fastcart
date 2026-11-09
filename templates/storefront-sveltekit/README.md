# FastCart SvelteKit storefront template

SvelteKit SSR template using `@sveltejs/adapter-cloudflare`.

```bash
pnpm install
pnpm dev
pnpm build:artifact
```

For a preview artifact:

```bash
pnpm build:artifact:preview
pnpm test:artifact:preview
```

SvelteKit rejects path-only `kit.paths.assets` (it requires an absolute URL), so
the preview build uses `kit.appDir = "_fastcart/preview-assets"` and packages the
adapter output at `client/_fastcart/preview-assets/`. The generated URLs match
FastCart's `/_fastcart/preview-assets/*` route. The production build still uses
the default `_app` directory and the `/_app/*` alias.
