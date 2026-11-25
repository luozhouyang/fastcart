# Live Chat example plugin

A complete storefront capability example:

- `plugin.json` declares one `livechat@1` storefront capability, one
  `storefront_component`, one nav entry, two admin slots, and one storefront embed.
- `worker/index.js` serves the capability routes under
  `/capabilities/livechat` and keeps the existing admin RPC method
  `conversations.list`.
- `storefront/boot.js` probes
  `/store/capabilities/livechat/session` with same-origin credentials and falls
  back to the legacy embed when the capability is unbound.
- `storefront/livechat-button.js` is the component entry. The admin exposes it
  as the `plugin:chat-widget:livechat-button` block, and a theme mounts it with
  `<fc-livechat-button label="Chat with us" position="bottom-right">`.
- `ui/settings.html` is the sandboxed iframe page; it speaks the `postMessage`
  protocol.
- `test/worker.test.mjs` exercises the worker handlers with `node:test`; it
  needs no bundler and no `workerd`.

## Layout

```text
plugin-chat/
├── plugin.json
├── worker/index.js
├── ui/settings.html
├── storefront/boot.js
├── storefront/boot.css
├── storefront/livechat-button.js
├── test/worker.test.mjs
└── test/component.test.mjs
```

## Declare the capability

The manifest contribution is independent of the embed. It names the capability,
pins a version, and describes the configuration an administrator may bind:

```json
"capabilities": { "storefront": ["store.read"] },
"provides": [
  {
    "kind": "storefront_capability",
    "id": "livechat",
    "version": 1,
    "title": "Live chat",
    "config_schema": {
      "type": "object",
      "properties": {
        "greeting": { "type": "string" },
        "accent": { "type": "string" },
        "position": { "type": "string", "enum": ["bottom-right", "bottom-left"] }
      },
      "additionalProperties": false
    }
  }
]
```

`config_schema` is enforced when an administrator binds the capability.
`additionalProperties: false` means only `greeting`, `accent` and `position` are
accepted. Declaring a contribution grants no execution rights by itself.

## Pack, install and enable

From the repository root:

```bash
node --experimental-strip-types packages/plugin-cli/src/cli.ts pack templates/plugin-chat
```

Inside the template you can also run `fastcart-plugin pack .` or validate it
without writing an archive with `fastcart-plugin doctor .`.

Upload `chat-widget-1.0.0.tar.gz` in **Admin → Plugins** (requires
`plugins.install`) and enable it (`plugins.write`). The API equivalents are
`POST /api/v1/admin/plugins/install` (multipart `archive`) and
`POST /api/v1/admin/plugins/chat-widget/enable`.

## Bind the capability

The admin capability catalog lists every contribution from enabled plugins and
the store's current bindings. It requires `plugins.read`:

```http
GET /api/v1/admin/capabilities
x-store-id: <store id>
```

```json
{
  "contributions": [
    {
      "plugin_id": "chat-widget",
      "plugin_name": "Live Chat",
      "plugin_version": "1.0.0",
      "id": "livechat",
      "version": 1,
      "title": "Live chat",
      "config_schema": { "type": "object", "...": "..." }
    }
  ],
  "bindings": []
}
```

Bind the capability to this plugin with `plugins.write`:

```http
PUT /api/v1/admin/capabilities/livechat/binding
x-store-id: <store id>
content-type: application/json

{
  "plugin_id": "chat-widget",
  "version": 1,
  "enabled": true,
  "config": {
    "greeting": "Hi! How can we help?",
    "accent": "#2563eb",
    "position": "bottom-right"
  }
}
```

The host checks that the installed manifest declares exactly `livechat@1`,
validates `config` against `config_schema`, and upserts one binding per
capability. `enabled` defaults to `true`. An unknown property, a bad type or an
unknown version is rejected. Delete the binding with
`DELETE /api/v1/admin/capabilities/livechat/binding`.

The Admin UI exposes the same catalog and binding as a form; it calls these
endpoints, so the API remains the source of truth.

Binding config and plugin settings are separate. The bound config is delivered
to the theme in its grant; the worker's session endpoint reads the
installation's own `settings` (edited from the plugin page) to choose the
greeting and accent, falling back to the constants above.

## The public capability route

Once the binding and the plugin are enabled, the storefront can call:

```text
GET  /store/capabilities/livechat/session
POST /store/capabilities/livechat/messages   {"message": "Hello"}
```

`session` returns the capability identity, the plugin's greeting/accent (or the
built-in defaults) and the resolved store:

```json
{
  "capability": "livechat",
  "version": 1,
  "greeting": "Hi! How can we help?",
  "accent": "#2563eb",
  "context": { "storeId": "store_123" }
}
```

`messages` accepts a JSON object with a string `message` of at most 2,000
characters and a body of at most 8 KiB. It returns an echo/ack payload; other
shapes return `400` and oversized bodies return `413`.

The host resolves the store from the request `Host`, never from a client
selector, then forwards `/store/capabilities/livechat` and
`/store/capabilities/livechat/{*path}` to the plugin worker at
`/capabilities/livechat...`. The worker receives only the trusted
host-injected context:

- `x-fastcart-store-id`, `x-fastcart-capability-id` and
  `x-fastcart-capability-version`;
- a short-lived `x-fastcart-context-token` with the reduced storefront scope
  set (storefront requests never carry KV, egress or admin scopes).

Every client-supplied `x-fastcart-*` header is stripped and replaced by the
host's value, so the worker must take the store from the injected header and
must not accept a store id from the path, query or body. The route is rate
limited per capability and client. Unbound capabilities, disabled bindings,
disabled plugins and bindings whose current manifest no longer declares the
bound version all return `404`. WebSocket upgrades are tunneled with the same
context token.

The bundled `storefront/boot.js` always uses the site-relative endpoint
(`/store/capabilities/livechat/...`), never an absolute third-party origin.

## The component contribution

The second contribution lets a page author place the chat button as a block
instead of relying on the embed:

```json
{
  "kind": "storefront_component",
  "id": "livechat-button",
  "title": "Live chat button",
  "category": "commerce",
  "entry": "storefront/livechat-button.js",
  "editor_manifest": {
    "title": "Live chat button",
    "fields": {
      "label": { "type": "text", "default": "Chat with us" },
      "position": { "type": "select", "default": "bottom-right" }
    },
    "defaultProps": { "label": "Chat with us", "position": "bottom-right" }
  },
  "requires": [{ "id": "livechat", "version": 1, "optional": true }]
}
```

Declaring a component requires the `storefront.component` storefront scope,
which the manifest requests. `GET /api/v1/admin/components` returns the entry as
a digest-scoped URL served by the plugin archive route, and the page editor
registers it as the namespaced block type
`plugin:chat-widget:livechat-button`. The saved block type names its provider,
so the page stays valid when the plugin is disabled; the editor then flags the
block as unavailable and the storefront renders a placeholder.

Because the component only _requires_ the capability optionally, a store
without a bound `livechat` capability still gets a button that explains chat is
unavailable rather than a broken block.

## What a theme receives

The storefront runtime resolves enabled bindings into capability grants and
passes them to the theme in the trusted `x-fastcart-capabilities` request
header. The host strips any client-supplied value first. The header is a
versioned envelope: `1.` followed by base64url-encoded JSON whose `grants` map
is keyed by capability id and carries `version`, `endpoint` and `config`. The
[Storefront SSR](https://docs.fastcart.dev/how-it-works/storefront-ssr/)
documentation owns the exact wire format.

A theme that needs the capability declares it in its manifest:

```json
{
  "requires": {
    "capabilities": [{ "id": "livechat", "version": 1 }]
  }
}
```

A non-optional requirement for a version blocks theme activation (and rollback)
with a conflict until an enabled binding at that version is backed by an
enabled provider. Add `"optional": true` when the theme can render without the
capability. A requirement without `version` accepts any bound version.

The theme can render from the grant's `config` directly; `position`, for
example, is intended for a theme that places the launcher itself. The bundled
boot script keeps the existing bottom-right placement and uses the session
response for `greeting` and `accent`.

## Graceful degradation

`storefront/boot.js` requests the session endpoint with
`credentials: "same-origin"`. When the route returns `404` (unbound or
disabled), it keeps the built-in greeting/accent and sends clicks to the legacy
`/apps/chat-widget/api/hello` path instead. The embed therefore still mounts on
stores that have not adopted the capability. If the binding disappears while
the panel is open, the first failed message falls back to the same legacy mode.

## Checks

```bash
node --check templates/plugin-chat/worker/index.js
node --check templates/plugin-chat/storefront/boot.js
node --test templates/plugin-chat/test/
node --experimental-strip-types packages/plugin-cli/src/cli.ts doctor templates/plugin-chat
```
