# FastCart Astro storefront template

Astro SSR reference for the universal workerd runtime.

```bash
pnpm install
pnpm dev
pnpm build:artifact
```

`build:artifact` packages the Cloudflare adapter worker and client assets as a
FastCart SSR artifact under `dist/artifacts/<digest>/`.

For a preview artifact whose client URLs use FastCart's preview asset prefix:

```bash
pnpm build:artifact:preview
pnpm test:artifact:preview
```

The preview build sets `FASTCART_ASSET_PREFIX=/_fastcart/preview-assets/`;
FastCart serves that path from the active preview artifact.
