# `@fastcart/plugin-cli`

Build and scaffold FastCart plugins in minutes.

```bash
pnpm --filter @fastcart/plugin-cli exec node --experimental-strip-types src/cli.ts init my-plugin --template minimal
cd my-plugin
pnpm --filter @fastcart/plugin-cli exec node --experimental-strip-types ../packages/plugin-cli/src/cli.ts pack .
```

The bin is `fastcart-plugin` once the package is linked:

```bash
fastcart-plugin init my-plugin --template chat
fastcart-plugin pack my-plugin
fastcart-plugin dev my-plugin --out .fastcart
fastcart-plugin doctor my-plugin
```

## Commands

| Command        | Behavior                                                                                                                                                                         |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `init <dir>`   | Scaffolds `minimal`, `admin`, or `chat` into an empty directory. `--force` overwrites.                                                                                           |
| `pack <dir>`   | Validates `plugin.json`, bundles the worker entry with esbuild (TypeScript + npm dependencies), copies `ui/`, `storefront/`, `assets/`, and writes `dist/<id>-<version>.tar.gz`. |
| `dev <dir>`    | Rebuilds the artifact when source files change. This is a rebuild loop, not a live workerd preview.                                                                              |
| `doctor <dir>` | Validates the manifest, entry source, referenced UI/storefront files.                                                                                                            |

## Artifact

`pack` produces a deterministic tar.gz:

- sorted paths, fixed mtime/mode, gzip level 9;
- `plugin.json` at the root;
- the bundled entry at `entry.path`;
- every referenced admin/storefront/settings file;
- TypeScript sources and `node_modules` are excluded.

The stdout prints the archive path and SHA-256 digest; the same source produces the same digest.

## Dev preview

`dev` needs a workerd binary. Resolution order: `--workerd <path>`,
`FASTCART_WORKERD`, or `workerd` on `PATH`.

- `/` lists admin and storefront entries.
- `/__preview/admin.html?entry=ui/settings.html` hosts an admin iframe and
  proxies RPC to `/apps/<id>/admin/<method>`.
- `/__preview/storefront.html?entry=storefront/boot.js` loads the storefront
  entry with a `FastCartExtensions` shim.
- `/apps/<id>/*` calls the plugin worker directly.

Editing a file rebuilds the worker and restarts workerd; there is no hot module
reload yet.

## Templates

- `minimal` — one worker, no UI.
- `admin` — admin nav entry, settings iframe, declared RPC.
- `chat` — admin settings + sidebar, storefront chat bubble, KV/egress scopes.

## Limits

- `dev` requires workerd and restarts it on every change.
- No minification, source maps, or signing.
- The package is private; publish it before external use.
