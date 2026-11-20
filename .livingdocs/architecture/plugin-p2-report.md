# Plugin System P2 Report: Runtime and Capability Bridge

**Date:** 2026-11-10
**Status:** Complete for the P2 core. Deviations are recorded below.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P1 report](plugin-p1-report.md), [P0 report](plugin-p0-report.md).

---

## 1. Delivered

### Runtime (`crates/plugin/src/runtime/`)

| Module       | Contents                                                                                                                                                                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `token.rs`   | HMAC-SHA256 context tokens (`fc_ctx_…`), 30 s lifetime, actor kinds, jti, constant-time verification, claim/signature/expiry tests                                                                                              |
| `config.rs`  | Static Cap'n Proto generation per runtime generation, embedded gateway/bridge assets, ephemeral loopback port, deployment-scoped runtime bearer token                                                                           |
| `process.rs` | Lazy workerd spawn with `--experimental`, readiness probe, per-plugin semaphore (8), request deadlines (10/30/30/60 s), 512 KB/8 MB response caps, watchdog restart after the deadline plus grace, health snapshot, log capture |
| `egress.rs`  | HTTPS-only allowlist, wildcard hosts, private/loopback/link-local/metadata blocking, DNS pinning, credential stripping, 15 s timeout, 5 MB response cap, daily quotas                                                           |

### Workerd assets

`crates/plugin/assets/gateway.js` and `crates/plugin/assets/bridge.js`:

- gateway resolves `{plugin}@{digest}`, fetches the bundle with the runtime token, builds the loader entry with `globalOutbound: null`, and forwards the request;
- bridge exposes only `/internal/plugins/v1/*`, strips `cookie`, store and plugin identity headers, and forwards to the host API;
- the plugin environment contains only `FASTCART_API` and `FASTCART_PLUGIN_ID`.

### HTTP surface

| Method         | Path                                              | Authentication                      |
| -------------- | ------------------------------------------------- | ----------------------------------- |
| GET            | `/apps/{plugin_id}` and `/apps/{plugin_id}/*`     | public storefront, Host-resolved    |
| POST           | `/api/v1/admin/plugins/{id}/rpc`                  | admin session/CSRF + `plugins.read` |
| GET            | `/api/v1/admin/plugins/runtime`                   | `plugins.read`                      |
| GET            | `/internal/plugins/v1/context`                    | context token                       |
| GET            | `/internal/plugins/v1/products`, `/products/{id}` | `catalog.read`                      |
| GET/PUT/DELETE | `/internal/plugins/v1/kv/{namespace}/{key}`       | `kv.read` / `kv.write`              |
| POST           | `/internal/plugins/v1/egress`                     | `network.egress`                    |

Internal routes are mounted only when `plugins.enabled=true`; the bundle endpoint keeps its separate runtime-token auth.

### SDK

`packages/plugin-sdk` (`@fastcart/plugin-sdk`) provides `createPluginContext(request)` with:

- `context()` for plugin/store/settings;
- `api(path, init)` for the bridged internal API;
- `kv.get/set/delete`;
- `egress(url, options)`;
- `definePlugin()` for typed handlers.

It sends the context token as `Authorization: Bearer …`; the bridge rewrites the virtual `http://api` origin to the host. `pnpm --filter @fastcart/plugin-sdk typecheck` passes.

### Configuration

| Key              | Env                              | Default                           |
| ---------------- | -------------------------------- | --------------------------------- |
| `workerd_binary` | `FASTCART_PLUGIN_WORKERD_BINARY` | `experiments/workerd/bin/workerd` |
| `runtime_dir`    | `FASTCART_PLUGIN_RUNTIME_DIR`    | `<plugins.root>/runtime`          |
| `context_secret` | `FASTCART_PLUGIN_CONTEXT_SECRET` | random per process                |
| `self_address`   | `FASTCART_PLUGIN_SELF_ADDRESS`   | derived from `server.host`/`port` |

`config/config.example.toml` documents the section.

---

## 2. Verification

| Suite                                                                              | Result                                                                   |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `cargo test -p fastcart-plugin`                                                    | 43 tests (token, inject, streaming, dispatch, config, archive, manifest) |
| `cargo test -p fastcart-plugin --test runtime_workerd`                             | 2 real-workerd tests                                                     |
| `cargo test -p fastcart-api --test admin_plugins`                                  | 19 tests                                                                 |
| `cargo test -p fastcart-api` (all suites)                                          | passed                                                                   |
| `cargo clippy -p fastcart-plugin -p fastcart-api -p fastcart-server --all-targets` | clean                                                                    |
| `cargo fmt --all -- --check`                                                       | clean                                                                    |
| `pnpm --filter @fastcart/plugin-sdk typecheck`                                     | clean                                                                    |

End-to-end checks against the pinned workerd binary:

- a plugin bundle is fetched, compiled lazily, and executed;
- `env.FASTCART_PLUGIN_ID`, trusted store header and context token reach the plugin;
- the plugin calls `env.FASTCART_API` through the bridge and receives the host response;
- direct `fetch("https://example.com")` is blocked (`globalOutbound: null`);
- a synchronous infinite loop times out, returns an error, and the watchdog restarts the host; the next request succeeds;
- `/apps/{plugin}` forwards through store resolution into workerd and returns the plugin JSON.

API-level checks:

- context returns plugin, store locale/currency and settings;
- KV PUT/GET round trip; read-only token is forbidden on PUT; one request token serves several bridge calls;
- egress rejects a non-allowlisted host, plain HTTP, and an IP literal before any connection;
- disabled and unknown plugins are hidden on `/apps`.

---

## 3. Deviations: Closed After Review

The review pass closed every P2 deviation; WebSocket upgrades remain a P6 limitation:

| Deviation                 | Resolution                                                                                                                                                                                                                                                       |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1 deployment-scoped key  | Tokens are now signed with a key derived from `(store, plugin, digest)` and carry the digest; the verifier inspects the claims, derives the installation key, verifies, and checks the digest against the enabled installation (current or rollback target).     |
| D2 `jti` correlation only | The SDK adds `x-fastcart-call-id` to every bridge call; the host keeps a 60 s `(jti, call_id)` cache and rejects an exact replay with 401. A raw plugin that omits the header is still replayable within the 30 s token lifetime; the SDK is the supported path. |
| D3 UTF-8 egress only      | `POST /egress` accepts `body_base64`; `capabilities.network_auth` lists hosts that may receive `Authorization`; every other credential header is stripped.                                                                                                       |
| D4 no RPC allowlist       | The manifest has `rpc[]` (validated, max 50, unique); `POST /plugins/{id}/rpc` returns 404 for an undeclared method before contacting workerd.                                                                                                                   |
| D5 buffered responses     | `/apps` uses `forward_stream` with a capped body stream; the storefront injector scans chunks incrementally; WebSocket upgrades are tunnelled frame-by-frame.                                                                                                    |
| D6 catalog-only reads     | Added `GET /orders`, `/orders/{id}`, `/customers/{id}`, and `/pages/{handle}` with redacted order/customer projections.                                                                                                                                          |
| D7 logs only              | Added `fastcart_plugin_requests_total`, `fastcart_plugin_timeouts_total`, `fastcart_plugin_host_restarts_total`, `fastcart_plugin_egress_bytes_total`, and `fastcart_plugin_kv_bytes` to `/metrics`.                                                             |
| D8 explicit binary        | `plugins.enabled=true` now fails startup when `plugins.workerd_binary` is missing; the API no longer silently degrades.                                                                                                                                          |

Residual limitations:

- The replay cache is process-local; a multi-node deployment needs a shared store.
- Egress pins only the first resolved address; a host with mixed public/private answers is rejected, but round-robin re-resolution is not re-checked.
- Token rotation follows the digest: an upgrade invalidates in-flight tokens for the old digest after 30 s, except that the previous digest is accepted while it is the rollback target.

## 4. Known Gaps

| Gap                                         | Impact                                                            | Owner        |
| ------------------------------------------- | ----------------------------------------------------------------- | ------------ |
| WebSocket frames are capped at 1 MiB        | Larger messages close the tunnel                                  | P6           |
| No admin UI for deliveries/jobs             | API only; P5 delivered the workers                                | P7           |
| No `plugin_logs` persistence                | Workerd stdout goes to tracing                                    | P6           |
| Admin RPC permission intersection is coarse | Token carries actor permissions but endpoint names are not mapped | P3 follow-up |
| Replay cache is process-local               | Multi-node deployments                                            | P6           |
| Egress binary response is base64            | SDK decodes when needed                                           | P7           |
| Workerd crash counter is watchdog-only      | Crashes restart on the next request                               | P6           |

## 5. Next Phase Inputs

- P3 consumes the runtime manifest, iframe sandbox, and `POST /plugins/{id}/rpc`; add the `rpc` allowlist to the manifest before shipping the UI.
- P4 consumes `/apps/{plugin}` and the storefront scopes; both now stream, with WebSocket left to P6.
- P5 reuses `ContextClaims` with `actor.kind = event|job` and the existing delivery/job tables.
- P6 hardens quotas, metrics, logs, and the shared replay cache.
