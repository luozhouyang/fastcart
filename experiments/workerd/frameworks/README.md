# Phase 0 framework validation

Both fixture applications were built with their official Cloudflare integrations,
bundled into a single ES module, and served by the pinned `workerd` binary. This
validates the central assumption of the universal runtime: framework output is a
standard Fetch handler and does not need a FastCart-specific adapter at runtime.

## TanStack Start

- Template: TanStack Router `examples/react/start-basic-cloudflare`
- `@tanstack/react-start` `1.169.5` (the mirror did not have the example's
  `1.169.6`; the patch difference is not material)
- `@cloudflare/vite-plugin` `1.67.0`
- Vite `8.3.6`, Wrangler `4.158.0`
- Build: `pnpm exec vite build`
- Output: `dist/server/index.js` plus `dist/server/assets/*.js`
- Validation bundle: `esbuild dist/server/index.js --bundle --format=esm
--platform=neutral --tree-shaking=false --external:node:* --external:cloudflare:*`
- Workerd config: one module, `compatibilityDate = "2026-11-01"`; the Node
  compatibility flag is no longer required as of that date.
- Result: `GET /` returned the SSR document with the expected title and
  hydration metadata.

Important observations:

- The server output imports `node:events` and `node:stream`; workerd provides
  them through its Node compatibility layer when the compatibility date enables
  it.
- The output imports `cloudflare:workers`; the Worker receives bindings through
  the normal `env` object.
- The output is a module graph (`index.js` + `assets/*.js`), not a single file.
  The production artifact can either preserve the graph or bundle it; the
  Phase 0 validation bundled it with esbuild to keep the workerd config small.

## Astro

- Astro `7.4.7`
- `@astrojs/cloudflare` `14.4.6`
- Build: `pnpm exec astro build` with `output: "server"`
- Output: `dist/server/entry.mjs` plus `dist/server/chunks/*.mjs`; client
  assets under `dist/client`
- Validation bundle: `esbuild dist/server/entry.mjs --bundle --format=esm
--platform=neutral --tree-shaking=false --external:node:* --external:cloudflare:*`
- Workerd config: one module, `compatibilityDate = "2026-11-01"`.
- Result: `GET /` returned the SSR document with `FastCart Astro SSR` and the
  request path resolved by `Astro.url.pathname`.

Important observations:

- The adapter emits a Cloudflare-specific `wrangler.json` and expects bindings
  such as `ASSETS`, `IMAGES` and `SESSION` for full functionality. A plain page
  render worked without them; the production gateway must define which bindings
  FastCart supplies and reject artifacts that require unsupported bindings.
- Client assets live in `dist/client`; they are served by Rust in the target
  architecture, not by workerd.
- The adapter already sets immutable cache headers for `/_astro/*` in its
  `_headers` file; the Rust asset server will apply the same policy.

## Limits of this validation

- It validates rendering and the Worker contract, not the full FastCart artifact
  schema, deployment registry, streaming proxy, trusted headers or rollback.
- The bundling step is a validation convenience. Phase 2 decides whether the
  artifact packager keeps the module graph, emits a single bundle, or supports
  both.
- Framework fixtures are not yet committed as buildable apps; the Phase 5 and
  Phase 11 work replaces these notes with maintained templates and CI fixtures.
