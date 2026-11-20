# Plugin System P7 Report: Developer Experience

**Date:** 2026-11-10
**Status:** Implemented. Publishing the CLI package remains an operational step.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P6 report](plugin-p6-report.md), [SDK README](../../packages/plugin-sdk/README.md).

---

## 1. CLI

`packages/plugin-cli` (`@fastcart/plugin-cli`, bin `fastcart-plugin`):

| Command                                                | Behavior                                                                                                                                   |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `init <dir> --template minimal\|admin\|chat [--force]` | Copies a template, substitutes the directory-derived plugin id, refuses a non-empty directory without `--force`.                           |
| `pack <dir> [--out] [--no-bundle] [--keep-staging]`    | Validates `plugin.json`, bundles the entry with esbuild, copies UI/storefront/assets, writes a deterministic tar.gz and prints the digest. |
| `dev <dir> [--out] [--interval]`                       | Rebuilds the artifact on source changes. It is a rebuild watcher, not a live workerd preview.                                              |
| `doctor <dir>`                                         | Validates the manifest, entry source, and every referenced UI/storefront/settings file.                                                    |

### Bundling

`esbuild` runs with:

```text
bundle: true, format: "esm", platform: "neutral", target: "es2022"
external: ["node:*"], sourcemap: false, minify: false
```

TypeScript sources and `node_modules` are not copied into the artifact; the bundle replaces `entry.path`. The test fixture imports both a local `.ts` module and a fake npm package and asserts both are present in the archive.

### Dev preview

`dev` generates `.fastcart/dev`:

- `plugin.js` — esbuild bundle;
- `harness.js` — routes `/apps/<id>/*` to the worker and `/__preview/*` to a disk service;
- `dev.capnp` — workerd config with the main worker and an assets disk service;
- `assets/index.html`, `assets/admin.html`, `assets/storefront.html` — preview pages.

The admin host sends the same postMessage `init`/`rpc` protocol as the real
admin, proxying RPC to `/apps/<id>/admin/<method>`. The storefront page installs
a `FastCartExtensions` shim before loading the plugin entry.

Workerd resolution: `--workerd`, `FASTCART_WORKERD`, then `workerd` on `PATH`.
Each source change rebuilds and restarts the process.

### Deterministic artifact

`src/tar.ts` writes ustar headers with fixed mtime/mode and gzip level 9. Paths are sorted before packing. Two packs of the same source produce the same SHA-256.

## 2. Templates

Three scaffold sets live inside the CLI and in the repository:

| Template                   | Contents                                                    |
| -------------------------- | ----------------------------------------------------------- |
| `templates/plugin-minimal` | `plugin.json`, `worker/index.ts`, README.                   |
| `templates/plugin-admin`   | Admin nav + settings tab, `ui/settings.html`, declared RPC. |
| `templates/plugin-chat`    | Admin slots, storefront bubble, KV/egress scopes.           |

`init` uses the CLI-local copies so the package is self-contained; the repo copies are the examples referenced by the design.

## 3. Documentation

- `packages/plugin-cli/README.md` — quickstart, commands, artifact layout, limits.
- `packages/plugin-sdk/README.md` — context, API, KV, egress, logging, examples.

## 4. Verification

| Check                                                                | Result                                                                                                               |
| -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `pnpm --filter @fastcart/plugin-cli test`                            | 6 passed (pack bundle + npm dependency, determinism, init all templates, doctor, manifest validation, CLI init→pack) |
| `pnpm --filter @fastcart/plugin-cli typecheck`                       | clean                                                                                                                |
| `pnpm exec eslint packages/plugin-cli packages/plugin-sdk`           | clean                                                                                                                |
| `pnpm exec prettier --check packages/plugin-cli packages/plugin-sdk` | clean                                                                                                                |
| All three templates                                                  | init + doctor + pack succeed in tests                                                                                |

## 5. Five-minute flow

For a complete local demo without manual steps:

```bash
./scripts/plugin-demo.sh
```

It creates `.fastcart-demo/`, seeds a store and owner, packs and installs
`chat-widget`, then starts the API and admin on free-by-default demo ports
(18080 / 15174). Ctrl-C stops both. `--clean` resets the demo, `--no-plugin`
runs the admin only, and `--no-build` skips the Rust build on repeat runs.

Manual plugin flow:

```bash
fastcart-plugin init chat-widget --template chat
cd chat-widget
fastcart-plugin dev .      # preview at http://127.0.0.1:8787/
fastcart-plugin pack .
# Upload dist/chat-widget-1.0.0.tar.gz from Admin → Plugins
```

## 6. Limitations

| Limitation                                               | Impact                                                | Owner   |
| -------------------------------------------------------- | ----------------------------------------------------- | ------- |
| `dev` is not a workerd preview                           | Authors install the artifact to test runtime behavior | P8      |
| No minification/source maps                              | Larger bundles, harder debugging                      | P8      |
| No signing or registry publish                           | Artifacts are operator-uploaded                       | P8      |
| tar writer supports ustar paths only (≤255 split fields) | Very deep asset paths fail                            | P8      |
| Templates have no `package.json`/lockfile                | External authors must add their own tooling           | P8      |
| CLI package is private                                   | Publish `@fastcart/plugin-cli` before third-party use | Release |
