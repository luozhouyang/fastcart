# FastCart TanStack Start storefront template

Reference storefront for the universal workerd runtime. It builds a standard
Worker Fetch handler and packages it as a FastCart SSR artifact.

```bash
pnpm install
pnpm dev
pnpm build:artifact
```

`build:artifact` writes `dist/artifacts/<digest>/` with `fastcart-ssr.json`.
Point `ssr.artifact_path` at that directory and `ssr.binary` at the pinned
workerd binary.

For a preview artifact whose client URLs use FastCart's preview asset prefix:

```bash
pnpm build:artifact:preview
pnpm test:artifact:preview
```

The preview build sets `FASTCART_ASSET_PREFIX=/_fastcart/preview-assets/`;
FastCart serves that path from the active preview artifact.

Routes used by the runtime compatibility checks:

- `/` — SSR with a server function loader
- `/__fastcart/health` — runtime readiness
- `/echo` — POST body and trusted FastCart headers
- `/stream` — streamed response
- `/cookie` — duplicate `Set-Cookie` headers
- `/redirect` — 302
- `/boom` — error boundary
