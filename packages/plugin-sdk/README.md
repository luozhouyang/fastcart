# `@fastcart/plugin-sdk`

Typed helpers for FastCart plugin workers.

```ts
import { createPluginContext, definePlugin } from "@fastcart/plugin-sdk";

export default definePlugin({
  async fetch(ctx, request) {
    const value = await ctx.kv.get("chat", "last");
    if (value === null) await ctx.kv.set("chat", "last", "hello");
    return Response.json({ value, path: new URL(request.url).pathname });
  },
});
```

The host runs the plugin in workerd and injects a short-lived context token in
`x-fastcart-context-token`. The SDK forwards it as a bearer token through the
shared bridge.

## API

| Member                           | Purpose                                                                  |
| -------------------------------- | ------------------------------------------------------------------------ |
| `ctx.context()`                  | Plugin, store, scopes and published settings.                            |
| `ctx.api(path, init)`            | Raw call under `/internal/plugins/v1`. Adds the token and a per-call id. |
| `ctx.kv.get/set/delete(ns, key)` | Per-plugin, per-store KV with host quotas.                               |
| `ctx.egress(url, opts)`          | Allowlisted outbound HTTP through the host proxy. Returns base64 body.   |
| `ctx.log(...)`                   | Writes to console and to the durable plugin log.                         |

`definePlugin()` is an identity helper for typed `fetch`, `admin`, `events`, and
`jobs` handlers.

## Notes

- `ctx.egress` accepts `body` (UTF-8) or `bodyBase64` (binary).
- The Worker Loader environment already blocks direct `fetch`; always use `ctx.egress`.
- The SDK does not bundle itself in v1; the CLI does that at pack time.
