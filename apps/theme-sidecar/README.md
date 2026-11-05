# @fastcart/theme-sidecar

Node.js sidecar for the Fastcart `NodeRuntime`. It loads a trusted theme
bundle's server entry, calls its `render(payload)` export, and returns the
result over the JSON+base64 contract expected by
`crates/storefront-runtime/runtime-node`.

## Running

```bash
pnpm --filter @fastcart/theme-sidecar start
```

The sidecar does not import a theme at startup. Every `/render` request
resolves the active theme entry and renders it inside a short-lived worker
thread with a heap cap and a hard deadline. A runaway theme is terminated
instead of blocking other renders or `/health`; the trade-off is a small
per-request worker startup cost.

## Environment

| Variable         | Default         | Description                                                                                                                            |
| ---------------- | --------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `PORT`           | `8788`          | Listener port. Matches `storefront.node_url` in `config/config.example.toml`.                                                          |
| `HOST`           | `127.0.0.1`     | Listener host. Set `0.0.0.0` only in containers or behind a private network.                                                           |
| `THEME_ROOT`     | `<repo>/themes` | Directory containing bundles. Relative values resolve from the repo root.                                                              |
| `SIDECAR_SECRET` | unset           | When set, `POST /render` requires `Authorization: Bearer <secret>`. Always set this whenever the sidecar is reachable beyond loopback. |

`THEME_ROOT` and `bundle_path` compose: the entry is resolved at
`THEME_ROOT/<bundle_path>/<entry>`. For example the default root plus a
registered `bundle_path` of `default` loads `themes/default/dist/server/server.js`.
To keep the Rust default `bundle_path` values such as `themes/default`, start
the sidecar with `THEME_ROOT=.` (the repository root). Bundles must be
referenced with relative paths; traversal and absolute paths are rejected.

## HTTP contract

### `GET /health`

Returns `200 {"status":"ok"}`. Always unauthenticated.

### `POST /render`

Authenticated with the bearer secret when configured. Request body:

```json
{
  "request_id": "...",
  "store": { "id": "...", "domain": "...", "locale": "en", "currency": "USD", "timezone": "UTC" },
  "method": "GET",
  "url": "/",
  "headers": [["accept", "text/html"]],
  "body": null,
  "theme": {
    "id": "default",
    "version": "0.1.0",
    "runtime": "node",
    "bundle_path": "default",
    "entry": "dist/server/server.js",
    "settings": {},
    "assets_manifest": { "entries": { "client": "dist/client" } },
    "assets_base_url": "/api/v1/store/theme-assets"
  },
  "page": {
    "id": "...",
    "handle": "",
    "status": "published",
    "page_type": "home",
    "title": "Home",
    "seo": {},
    "body": { "schema_version": 1, "content": [], "zones": {} }
  },
  "data": {},
  "capabilities": {},
  "mode": "production",
  "timeout_ms": 10000
}
```

Response:

```json
{
  "status": 200,
  "headers": [["content-type", "text/html; charset=utf-8"]],
  "body": "PGh0bWw+..."
}
```

`body` is standard base64, or `null`. The decoded body is capped at 10 MiB and
the request body at 32 MiB. Errors are JSON
`{"error":{"code":"...","message":"..."}}` with a 4xx/5xx status; stacks and
filesystem paths are never returned to the caller.

### `GET /theme-assets/<bundle_path>/<client-relative-path>`

Serves files from the bundle's client directory (`assets.json`
`entries.client`, default `dist/client`). The longest path prefix containing a
`theme.json` is treated as the bundle path; the remainder is resolved inside
the client directory with symlink and traversal protection.

To route browser assets through this sidecar, point the theme's
`assets_base_url` at `http://<sidecar-host>:8788/theme-assets/<bundle_path>`.
When the Rust API serves assets itself it sends
`/api/v1/store/theme-assets`, whose route already knows the active bundle, so
the renderer appends only `client.js`.

## Theme bundle contract

A bundle produced by `themes/default` looks like:

```
themes/default/
├── theme.json            # runtime "node", entry "dist/server/server.js"
├── assets.json           # client: "dist/client", server: "dist/server/server.js"
├── editor.manifest.json
├── dist/
│   ├── client/client.js  # browser hydration entry
│   ├── client/editor.js  # admin editor component registry
│   └── server/server.js  # exports render(payload)
```

Build it with:

```bash
pnpm --filter @fastcart/theme-default build
```
