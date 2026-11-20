# Fastcart Docs

The documentation site for Fastcart, built with
[Cloudflare Nimbus](https://nimbus-docs.com) on Astro 7. Pages are MDX files under
`src/content/docs/`; the folder tree is both the URL structure and the sidebar.

## Run

From the repository root:

```bash
pnpm install
pnpm dev:docs
```

The dev server runs on <http://localhost:4321>. You can also run it from this directory with
`pnpm dev`.

## Write

- Add a page by creating `src/content/docs/<slug>.mdx` with at least a `title` in its frontmatter.
- Create folders to group pages; a folder's `index.mdx` is the group landing page.
- Set `sidebar.order` and `sidebar.label` in frontmatter to control navigation.
- Components (`Aside`, `Steps`, `Tabs`, `Card`, and others) are available in every MDX file
  without imports. See `src/components.ts`.

[AGENT.md](./AGENT.md) is the canonical guide for editing, auditing, and upgrading this site, and
`CLAUDE.md` delegates to it. The `nimbus.json` provenance file must stay committed: upgrades use it
to decide what you own.

## Check and build

```bash
pnpm --filter @fastcart/docs typecheck   # astro check (types + content)
pnpm --filter @fastcart/docs lint:docs   # Nimbus authoring lint
pnpm --filter @fastcart/docs build       # static output in dist/
pnpm --filter @fastcart/docs preview     # serve the production build locally
```

For the full preflight (environment, structure, authoring, and types):

```bash
pnpm --filter @fastcart/docs exec nimbus-docs check
```

## Deploy

The site is static and deploys to Cloudflare Workers Static Assets with Wrangler. Before the first
deploy, set the production origin in `astro.config.ts` (`site`) and the Worker name in
`wrangler.jsonc`. Then:

```bash
pnpm deploy:docs
```

This runs `astro check`, `astro build`, and `wrangler deploy`. To try the static output through
Wrangler locally, run `pnpm --filter @fastcart/docs preview:cf`.

## Notes for the monorepo

This app is a pnpm workspace package (`@fastcart/docs`), so dependencies are installed from the
repository root and a single `pnpm-lock.yaml` is used. The Nimbus scaffolder normally writes a
project-local `pnpm-workspace.yaml` for its dependency build-script settings; this app does not
ship one because the root workspace owns pnpm configuration. `sharp` and `workerd` install scripts
stay disabled, as their prebuilt binaries come from optional dependencies.
