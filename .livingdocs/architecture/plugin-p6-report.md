# Plugin System P6 Report: Security and Observability

**Date:** 2026-11-10
**Status:** Complete for the P6 scope, including WebSocket upgrades.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P5 report](plugin-p5-report.md), [P2 report](plugin-p2-report.md).

---

## 1. Delivered

### Plugin logs

| Surface                                  | Behavior                                                                                                                                 |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /internal/plugins/v1/log`          | Context-token authenticated; accepts `{level, message, fields}`, caps the message at 4 KB, rejects unknown levels, writes `plugin_logs`. |
| `GET /api/v1/admin/plugins/{id}/logs`    | Last 200 lines, `plugins.read`.                                                                                                          |
| `DELETE /api/v1/admin/plugins/{id}/logs` | Clears one plugin's logs, `plugins.write`.                                                                                               |
| SDK `ctx.log()`                          | Writes to `console` and fire-and-forget posts to the host; failures are ignored.                                                         |
| Retention                                | Maintenance deletes lines older than seven days.                                                                                         |

`console.log` output from workerd is still captured as tracing only; structured `ctx.log` is the durable path.

### RPC permission mapping

`POST /api/v1/admin/plugins/{id}/rpc` now maps the method prefix to the acting user's permission before forwarding:

| Prefix       | Permission                                   |
| ------------ | -------------------------------------------- |
| `orders.`    | `orders.read`                                |
| `products.`  | `products.read`                              |
| `customers.` | `customers.read`                             |
| `settings.`  | `settings.read`                              |
| `plugins.`   | `plugins.read`                               |
| other        | only the manifest allowlist + `plugins.read` |

### Mount tokens

Admin iframe mount tokens now live for one hour instead of five minutes. The token is only needed to load the HTML entry; a subsequent page reload fetches a fresh manifest and a fresh token. Tokens remain installation-derived, store/plugin/digest-bound, and single-purpose (`ui.mount`).

### Health

`GET /health/plugins` returns runtime state without authentication:

```json
{
  "status": "ok|unavailable|disabled",
  "running": true,
  "generation": "...",
  "port": 1234,
  "restarts": 0,
  "uptime_seconds": 12
}
```

### WebSocket upgrade

`/apps/{plugin}` now detects an HTTP/1.1 WebSocket upgrade and:

- computes the RFC 6455 accept key for the browser;
- opens a raw client handshake to workerd at `/__plugin/{id}@{digest}{path}` with the store, context token and gateway token;
- proxies frames in both directions, rewriting the mask bit (`browser → workerd` masked, `workerd → browser` unmasked);
- caps each frame at 1 MiB and forwards close/ping/pong frames unchanged.

No WebSocket client crate is required; the tunnel is implemented with `hyper::upgrade`, `sha1`, `base64` and tokio I/O.

### Streaming

- `/apps/{plugin}` uses `forward_stream`; the body is streamed with an 8 MB cap, and the per-plugin permit is held for the stream lifetime.
- Storefront HTML injection uses `StreamInjector` with a 32 KB tail; markers split across chunks are found, and head injection happens as soon as `</head>` is seen.
- Admin RPC remains buffered at 512 KB by design.

### Metrics

Existing `/metrics` plugin series from the P2 closure remain: requests, timeouts, restarts, egress bytes, KV bytes.

## 2. Verification

| Check                                               | Result                                     |
| --------------------------------------------------- | ------------------------------------------ |
| `admin_plugins::internal_api_context_kv_and_scopes` | log write + admin read + `/health/plugins` |
| `runtime_workerd::streams_plugin_responses`         | chunked body streamed through `/apps` path |
| `inject::stream` unit tests                         | one-byte chunks, fallback, sentinel        |
| Full affected suites                                | passed                                     |

## 3. Limitations

| Limitation                          | Impact                                    | Owner |
| ----------------------------------- | ----------------------------------------- | ----- |
| `console.log` is tracing-only       | Durable logs require `ctx.log`            | P7    |
| Metrics are unlabeled counters      | No per-plugin breakdown in Prometheus yet | P7    |
| No per-plugin log search or tail UI | API only                                  | P7    |
| No audit row for watchdog restarts  | Restart metric exists                     | P7    |
