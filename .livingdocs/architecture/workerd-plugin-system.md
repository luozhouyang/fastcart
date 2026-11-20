# FastCart Plugin System (workerd Execution Plane)

## Architecture Research and Implementation Plan

**Status:** Draft for review. This document is the only change: no product code has been modified.
**Runtime:** Cloudflare workerd (pinned in this repository at `v1.20261109.1`)
**Host:** Rust (`fastcart-serve`)
**Extension surfaces:** Admin UI, Storefront UI, server-side API / events / jobs
**Goal:** plugin code never runs inside the host process; plugins interact with the platform only through a controlled API.

---

## Decision Log

**2026-11-10 — plugin authorship, runtime substrate, and v1 scope**

The following decisions are settled and are reflected throughout this document.

| #   | Decision                                                                           | Consequence                                                                                                                                                                                                                                                                             |
| --- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Plugins are authored by **third-party JavaScript/TypeScript developers**.          | workerd is the runtime substrate. TypeScript, npm-at-build-time, and standard Web APIs are first-class requirements. Wasm is not a plugin runtime.                                                                                                                                      |
| 2   | **All plugins share a single workerd process and a single Worker Loader.**         | Per-plugin OS-level process isolation is explicitly **deferred**; see [Roadmap](#roadmap-deferred-work). The loader is the only isolation boundary in v1.                                                                                                                               |
| 3   | **Plugin signing is not part of v1.**                                              | No publisher keys, no signature verification, no revocation feed in v1. Install consent and operator review are the only provenance controls. Signing is deferred.                                                                                                                      |
| 4   | **`nodejs_compat` is enabled**, with an explicit allowlist.                        | `Buffer`, `process`, `node:events`, `node:path`, `node:util`, and WebCrypto-backed `node:crypto` are usable. No filesystem or raw-socket capability is granted.                                                                                                                         |
| 5   | **Theme-side injection markers are implemented in v1**, with a `</body>` fallback. | Themes may declare `<!--fastcart:extensions-head-->` / `<!--fastcart:extensions-body-->`, or the JSX-safe `<script type="application/fastcart-extensions" data-slot="…">`. Default templates ship with markers; themes without them still receive bundles before `</head>` / `</body>`. |
| 6   | **All plugin Rust code lives under `crates/plugin/`.**                             | No new top-level `crates/plugin-host`. The existing manifest registry becomes a module of the same crate. HTTP handlers remain in `crates/api` (presentation layer).                                                                                                                    |
| 7   | **Living documentation is written in English.**                                    | This document is the reference. Other `.livingdocs` files were already English.                                                                                                                                                                                                         |

**P0 status (2026-11-10):** executed. 45 checks, 27/27 gates passed; the loader, capability boundary, injector, WebSockets, streaming, and module graphs are validated. Loader `limits` are not enforced, and a synchronous loop blocks other plugins for the duration (2.9 s measured). Results and required changes are in [plugin-p0-report.md](plugin-p0-report.md).

**2026-11-10 — review decisions (approved)**

| #   | Decision                                                                                                                                                                                                                                                                                    | Effect                                                                                                                                                       |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 8   | **Watchdog restarts are accepted.** A request that exceeds its deadline may trigger a restart of the whole plugin host.                                                                                                                                                                     | In-flight requests from other plugins fail with 504; an audit record and metric are written. This is the only reliable way to stop a synchronous loop in v1. |
| 9   | **Default limits are approved as proposed:** 8 concurrent requests per plugin; 10 s admin, 30 s storefront, 30 s event, 60 s job; 512 KB RPC and 8 MB storefront responses; 10 MB and 100k keys of KV per plugin per store; 1,000 egress requests and 100 MB per day; 10 allowlisted hosts. | Values are configurable; P2 load tests may tune them.                                                                                                        |
| 10  | **The v1 admin target set is narrowed to six** and grows release by release.                                                                                                                                                                                                                | 6.1 lists the six; the rest are reserved and rejected for now.                                                                                               |
| 11  | **Write operations are deferred.** v1 plugins read business data, use KV, consume events, and run jobs.                                                                                                                                                                                     | `catalog.write`, `orders.write`, and similar scopes are reserved and rejected.                                                                               |
| 12  | **No generic `plugin_records` table in v1.**                                                                                                                                                                                                                                                | Session and message data lives in KV; revisited after the chat pilot.                                                                                        |
| 13  | **5.4 constraints accepted:** 4 MB per-module cap; node built-ins are importable but not capable.                                                                                                                                                                                           | `worker_threads` behavior is verified in P1 hardening, not a P1 blocker.                                                                                     |

---

# 0. Summary

1. **The direction fits the existing architecture.** `crates/ssr-workerd` already proves out workerd process supervision, Cap'n Proto generation, health checks, a streaming gateway, and internal tokens. The plugin runtime reuses that operational model.
2. **workerd is the plugin execution plane; Rust is the plugin capability plane.** A plugin receives exactly two capabilities: a loader-provided `env` with a shared bridge fetcher, and a plugin id. It cannot reach the database, choose a store, or reach the internet directly.
3. **Plugins are loaded at runtime through the Worker Loader.** `LOADER.get("{plugin}@{digest}", …)` means install, upgrade, enable, disable, and rollback never regenerate configuration and never restart a process. This removes the main "inflexibility" objection to workerd.
4. **Admin UI uses sandboxed iframes plus a `postMessage` bridge**, not build-time React bundling (Medusa's approach). The host owns a registry of named targets; plugins declare targets in their manifest.
5. **Storefront uses one boot script and custom elements with Shadow DOM.** A chat bubble is the equivalent of a Shopify app embed, injected at the theme marker (or before `</body>` when a theme has no marker) and mounted as a fixed-position element.
6. **Egress is closed by default.** In the loader, each plugin gets `globalOutbound: null`; global `fetch` throws. Outbound traffic goes through a host egress proxy with an allowlist and SSRF protection. This was verified locally.
7. **Do not co-locate SSR and plugins in one workerd process.** Their lifecycle, configuration, and trust domains differ. Plugins get their own process (≈85 MB RSS measured).
8. **Wasm is not rejected; it is scoped down.** It remains a future option for deterministic host-internal hooks (discounts, pricing, validation), not a plugin runtime. workerd can load Wasm modules inside a plugin if a plugin needs one.

Measured locally (Linux x86_64, loopback, pinned workerd):

| Measurement                                           | Result                                       | Notes                                    |
| ----------------------------------------------------- | -------------------------------------------- | ---------------------------------------- |
| workerd cold start                                    | **61.8 ms**                                  | P0 measurement, spawn to first response  |
| Gateway → plugin → bridge → external HTTP → mock host | **p50 0.31–0.45 ms / p95 0.50–0.59 ms**      | P0, 500 sequential requests              |
| RSS with 4 services                                   | **85.7 MB**                                  | gateway + plugin + bridge + external     |
| RSS with 10 loaded plugins                            | **+17.5–17.6 MB** (75.9–76.4 → 93.5–94.0 MB) | P0, ≈1.75 MB per plugin                  |
| WebSocket passthrough                                 | 101 Switching Protocols                      | Usable for a chat plugin                 |
| Global `fetch` without an outbound policy             | **200 — allowed by default**                 | Must be closed explicitly                |
| `globalOutbound = "none"` (static service)            | Startup failure: no such service             | Static services need a `blocked` service |
| Worker Loader `limits`                                | Accepted, **not enforced** in this build     | Host-side enforcement required           |

Full P0 results: [plugin-p0-report.md](plugin-p0-report.md).

---

# 1. Research: How Shopify and Medusa Implement Plugins

## 1.1 Shopify

Shopify separates four layers: **App (identity and authorization) → Extensions (UI and logic) → Theme app extensions (storefront) → Functions (Wasm backend logic)**.

### 1.1.1 App layer

- An app is installed through OAuth and receives **access scopes** (for example `write_products`, `read_orders`).
- It reads and writes data through the Admin GraphQL API and subscribes to events through webhooks.
- Everything is versioned together as an **app version**: `deploy` publishes a version and rollback returns to a previous one.

### 1.1.2 Admin UI extensions

An extension has three parts:

- **Targets** declare where it appears, for example `admin.product-details.block.render`, `admin.order-details.action.render`, `admin.product-index.selection-action.render`, or `admin.settings.validation.render`.
- **Target APIs** expose target-specific data and actions (current resource, close modal, i18n, toast).
- **Web components** are the host-provided building blocks (`s-*`), which keep the extension visually consistent with the admin.

Implementation details worth copying:

- **Remote DOM.** Extension code renders a remote tree that the host maps to real components. The documentation states that admin UI extensions are rendered using Remote DOM.
- **Sandboxed iframe.** Extension code runs in an isolated frame and never touches host DOM directly.
- **Direct API access.** `fetch("shopify:admin/api/graphql.json")` is proxied by the host with the app's authorization, so no developer backend is required for simple calls.
- **Conditional rendering.** `*.should-render` targets let an extension decide whether it appears before any UI is rendered.
- **Bundle limits.** UI extensions are capped at 64 KB compressed (128 KB for full-page customer account extensions).
- **Versioning.** Extensions ship as part of an app version and roll back with it.

**Takeaway:** targets are declarative, named, and typed; the host owns rendering and security; data access is proxied by the host.

### 1.1.3 Storefront: theme app extensions

A theme app extension bundles **app blocks** (merchant-positioned content), **app embed blocks** (global overlays/scripts), and **assets**.

- **App block:** `{% schema %}` with `"target": "content"`. The merchant drags it into a supporting section, so placement is merchant-controlled and dynamic sources can be bound.
- **App embed block:** `"target": "head" | "body" | "compliance_head"`. Shopify renders and injects it before `</head>` and `</body>`. The documentation explicitly names chat bubbles as the canonical app embed use case.
- **Loading:** the `javascript` entry is added to `<head>` and included once per page; `enabled_on` restricts it to templates.
- **Assets:** the `assets/` directory is served from Shopify's CDN.
- **Limits:** 10 MB total, 30 blocks, 100 KB of Liquid; theme app extensions never render on checkout pages.
- **Activation visibility:** `app.extensions()` reports whether a block is active on the published theme.

**Takeaway:** "global overlay" and "merchant-positioned block" are two different extension points, and both are needed.

### 1.1.4 Shopify Functions (Wasm)

- Input is a JSON document produced by a GraphQL input query; output is a JSON document of operations Shopify should perform.
- Logic compiles to **WebAssembly**; Rust is the recommended language for performance.
- A function cannot be called by URL, cannot make network requests, and is invoked by Shopify at the relevant point in the business flow.
- Configuration UI is a separate admin target.

**Takeaway:** Wasm is the right tool for synchronous, deterministic, hot-path hooks. It is the wrong tool for full applications.

## 1.2 Medusa

### 1.2.1 Plugin model

- A plugin is an **npm package** (since v2.3.0) registered in `medusa-config.ts`:
  ```ts
  plugins: [{ resolve: "@myorg/plugin-name", options: {} }];
  ```
- It may contain modules, API routes, workflows, workflow hooks, links, subscribers, scheduled jobs, and **admin extensions**.
- Distribution is `medusa plugin:build` → `npm publish`; local development uses `plugin:publish` / `plugin:develop` (yalc plus a watcher).

### 1.2.2 Admin extensions

- The admin is a **Vite + React Router SPA**.
- **Widgets:** `src/admin/widgets/*.tsx` export a React component and `defineWidgetConfig({ zone: "product.details" })`; detail-page widgets receive `DetailWidgetProps<T>` as `data`.
- **UI routes:** `src/admin/routes/.../page.tsx` plus `defineRouteConfig({ label, icon })`, which adds a sidebar entry.
- **Injection zones:** fixed string constants (`product.details`, `order.details.side`, `customer.list`, …) with a published reference list. Since v2.17.2 the `.before`/`.after` suffixes are deprecated in favor of layout configurations, and plugins can register custom zones.
- **Build-time bundling:** `medusa build` compiles `src/admin` and the admin extensions of installed plugins into the admin bundle. `plugin:develop` provides HMR.

### 1.2.3 Key differences

- Medusa plugins run **in the same Node process** as the application, with full permissions: database access, arbitrary network, arbitrary `require`. There is no sandbox and no capability declaration.
- The storefront has no extension system. A storefront is a separate Next.js app that calls the Store API; a chat bubble is simply a component in that app.
- This model is well suited to **first-party or trusted code reuse**, not to merchant-installed third-party plugins.

## 1.3 Comparison

| Dimension              | Shopify                                           | Medusa v2                                      | FastCart (this design)                                                |
| ---------------------- | ------------------------------------------------- | ---------------------------------------------- | --------------------------------------------------------------------- |
| Plugin artifact        | Platform-hosted app + extensions                  | npm package, compiled at build time            | Immutable bundle: manifest + worker modules + UI assets               |
| Server execution       | Shopify-hosted (Functions in Wasm)                | Same Node process                              | Shared workerd process, one loaded worker per plugin                  |
| Sandbox                | Yes (Remote DOM + platform isolation)             | None                                           | Process + V8 isolate + capability bindings                            |
| Admin injection        | Declarative targets + Remote DOM + web components | Declarative zones + React compiled into bundle | Declarative targets + sandboxed iframe + `postMessage`                |
| Admin extension timing | Runtime                                           | Build time                                     | Runtime                                                               |
| Storefront injection   | Theme app extension (block / embed)               | None                                           | Embed at end of body + custom elements                                |
| Storefront assets      | Platform CDN                                      | None                                           | Rust static service, content-addressed                                |
| Versioning             | App version                                       | npm semver                                     | Artifact digest + installation record + rollback                      |
| Authorization          | OAuth scopes + host-proxied API                   | None (full process access)                     | Capability scopes + short-lived tokens + user-permission intersection |
| Multi-tenant           | Platform side                                     | Single instance                                | Per-store installation, token bound to store                          |

## 1.4 What we adopt and what we reject

**Adopt**

- Shopify's **declarative targets, host-owned rendering, host-proxied API, conditional rendering, and whole-version releases**.
- Shopify's **app block / app embed split**: `embed` for global overlays, `slot` for theme-positioned content.
- Medusa's **readable zone naming** and its packaging/CLI developer experience.
- Medusa's **custom zones** idea (a plugin page can expose its own slots) as later work.

**Reject**

- Compiling third-party React into the admin bundle (Medusa). It places third-party code in the host page with no permission or fault boundary.
- Running plugins in the host process (Medusa). One OOM or infinite loop would take down the store.
- Wasm as the only plugin form (Shopify Functions' boundary). No network, no UI, no streaming: a chat plugin is not feasible.
- Letting a theme import plugin code directly. Themes are independent build artifacts; plugins must be installable at runtime.

---

# 2. Goals and Non-Goals

## 2.1 Goals

1. Merchants can install, enable, configure, upgrade, roll back, and remove plugins without restarting `fastcart-serve` or the plugin runtime process.
2. Admin pages expose a fixed set of named slots (nav, list pages, detail sidebars, settings tabs, command palette, global banner).
3. Storefront pages can load global overlays/scripts (chat bubble) without modifying the theme.
4. Plugin servers can read business data (scope-limited), use KV, subscribe to events, register jobs, and call allowlisted external hosts.
5. A failing plugin never returns 500 for the storefront and never blocks the admin shell.
6. Multi-store isolation: installation, settings, secrets, KV, logs, and quota are per store.
7. Observability: per-plugin requests, latency, errors, restarts, KV usage, and egress.
8. Developer experience for TypeScript authors: `init → dev → pack → install`, npm at build time, no Node at runtime.

## 2.2 Non-Goals (v1)

- A public plugin marketplace, publisher accounts, signing, and review pipeline. Signing is deferred.
- Per-plugin OS-level process isolation. Deferred; see [Roadmap](#roadmap-deferred-work).
- Plugin-owned database tables or migrations. KV plus business metadata only.
- Plugins modifying host layout, replacing core components, or intercepting core business flows. v1 is additive only.
- Direct plugin-to-plugin calls.
- Server-rendered storefront slots. v1 mounts client-side only.
- Multi-node or multi-instance deployment, consistent with the single-node architecture.

---

# Roadmap: Deferred Work

These items are explicitly out of scope for v1 and are recorded so that the architecture does not depend on them.

| Item                                                  | Decision | Why it is safe to defer                                                                                                | What must remain true                                                                                                                     |
| ----------------------------------------------------- | -------- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Per-plugin OS process isolation (`isolation=process`) | Deferred | All v1 plugins are operator-installed and reviewed; the loader already provides a V8 isolate and a capability boundary | The runtime API must not assume a process per plugin; the host must be able to add a second process later without changing the plugin SDK |
| Plugin signing and revocation                         | Deferred | No marketplace and no untrusted distribution in v1                                                                     | The manifest must reserve `signatures[]`; the loader must be able to reject by plugin id or digest                                        |
| Server-rendered storefront slots                      | Deferred | Client-side mounting covers the chat use case                                                                          | The storefront manifest should keep a `kind` field so `server` can be added                                                               |
| Wasm hooks                                            | Deferred | Not needed for third-party JS plugins                                                                                  | No host API may expose Wasm-only semantics                                                                                                |

---

# 3. Architecture

## 3.1 Process and trust boundaries

```text
                        ┌──────────────────────────────────────────────┐
   Browser ──HTTPS──▶   │ fastcart-serve (Rust, single process)        │
                        │                                              │
                        │  /api/v1/admin/*       admin API + RBAC/CSRF │
                        │  /api/v1/store/*       public store API      │
                        │  /internal/*           internal namespaces   │
                        │  /apps/{plugin}/*      plugin public routes  │
                        │  /_fastcart/*          static + plugin assets│
                        │                                              │
                        │  ┌────────────────┐  ┌────────────────────┐  │
                        │  │ SSR gateway     │  │ Plugin host manager │  │
                        │  └───────┬────────┘  └─────────┬──────────┘  │
                        └──────────┼─────────────────────┼─────────────┘
                                   │ loopback            │ loopback (one port per generation)
                        ┌──────────▼──────────┐  ┌───────▼──────────────────────────┐
                        │ workerd: SSR group   │  │ workerd: plugin host (separate)   │
                        │  (theme artifact)    │  │  --experimental                   │
                        │                      │  │  gateway (only socket)            │
                        │                      │  │   ├─ LOADER.get(id@digest)         │
                        │                      │  │   │    ├─ plugin:chat (isolate)    │
                        │                      │  │   │    └─ plugin:reviews           │
                        │                      │  │   └─ BRIDGE (shared)               │
                        │                      │  │        └─ external ─▶ Rust API     │
                        └──────────────────────┘  └───────────────────────────────────┘
```

Three invariants:

1. **A plugin can only reach the outside through loader-injected `env`:** one shared bridge fetcher and one plugin id. There are no other bindings.
2. **The bridge has a hard-coded upstream.** It forwards only to `http://commerce/internal/plugins/v1/*`; a plugin cannot reach another path or host through it.
3. **The loader sets `globalOutbound: null` for every plugin.** Global `fetch` throws. Internet access is only available through the host egress proxy.

## 3.2 Key decisions

| #   | Decision                                                                                      | Rationale                                                                                                | Alternative considered                                                                                                     |
| --- | --------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| D1  | Plugin code runs in a **separate workerd process**, never in the host                         | Crash, runaway loop, and OOM containment; matches the SSR deployment model                               | Embedding V8 in the host: we would own the runtime and lose process isolation                                              |
| D2  | **One shared loader** loads every plugin as `{id}@{digest}`                                   | Measured ~1.1 MB per plugin; install and upgrade need no config change or restart; digest is the version | Per-plugin process (~85 MB each, infeasible); static services (restart on install, superseded by the loader)               |
| D3  | Plugins and host communicate over **HTTP/JSON** (fetch style)                                 | Portable to Cloudflare/Miniflare/wrangler; no custom RPC protocol; easy to debug                         | workerd RPC (`WorkerEntrypoint`): nicer calls, but a workerd-specific API and still HTTP across the process boundary       |
| D4  | Admin UI uses **sandboxed iframes**, not Remote DOM                                           | Low implementation cost, strong isolation, any frontend framework                                        | Remote DOM: native look and feel, but requires a full remote component protocol (deferred)                                 |
| D5  | Storefront injects **one boot script**; plugins mount **custom elements with Shadow DOM**     | One injection point, style isolation, framework-agnostic themes                                          | One script tag per plugin: N injections and conflicts; iframe overlay: stronger isolation but extra messaging              |
| D6  | **Theme markers first**, with a `</body>` fallback                                            | Precise placement, no false positives, streaming-friendly; third-party themes without markers still work | Only the fallback: fragile with inline `</body>` strings and streamed tails. Only markers: themes without them get nothing |
| D7  | Plugin API uses **capability scopes plus short-lived signed tokens**                          | Token binds store, plugin, actor, and scopes; Rust validates per endpoint                                | Long-lived static token: a leak becomes permanent access                                                                   |
| D8  | Egress only through a **host proxy** (domain allowlist, SSRF protection)                      | Single auditable, rate-limited, revocable exit                                                           | Giving the plugin internet access: open by default, unverifiable                                                           |
| D9  | Artifacts are **content-addressed (digest)**; upgrade switches digest; rollback switches back | Same model as the SSR artifact store; cacheable and auditable                                            | Overwriting files in place: no rollback and cache invalidation problems                                                    |
| D10 | v1 has **no plugin-owned SQL or migrations**                                                  | Avoids exposing the platform schema; KV covers most cases                                                | One database per plugin: backup, migration, deletion, and quota complexity                                                 |

## 3.3 Why not Wasm (response to the earlier proposal)

| Requirement                                     | Pure Wasm                                      | workerd                                       |
| ----------------------------------------------- | ---------------------------------------------- | --------------------------------------------- |
| A chat plugin calling an LLM or third-party API | Host functions and a custom ABI for every call | Native `fetch` through the host proxy         |
| Streaming, SSE, WebSocket                       | Implement from scratch                         | Native (101 passthrough measured)             |
| Plugin UI                                       | Not supported                                  | HTML/JS assets, iframe or custom elements     |
| Crypto, randomness, time, URL                   | One host function each                         | Standard Web APIs                             |
| Events and jobs                                 | Host-driven, workable                          | Host-driven, with async and streams available |
| Cold start and latency                          | Faster, deterministic                          | Already sufficient (p50 0.43 ms per hop)      |
| Deterministic computation (discounts, pricing)  | Best fit                                       | Workable, less controllable                   |

**Target shape:** workerd carries the plugin body; a plugin may still ship a Wasm module for pure computation and load it inside workerd. Wasm keeps its determinism advantage without constraining I/O.

---

# 4. Runtime Design

## 4.1 Configuration and plugin loading

The primary path is the **Worker Loader**, which is built into the pinned workerd and requires `--experimental`. The configuration is **static and independent of the number of plugins**: plugins are loaded at runtime by a host loader worker, so installing, upgrading, enabling, disabling, and rolling back a plugin never regenerate configuration and never restart the process.

```capnp
using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "gateway",  worker = .gatewayWorker),
    (name = "bridge",   worker = .bridgeWorker),
    (name = "commerce", external = (address = "127.0.0.1:8080", http = ())),
  ],
  sockets = [
    (name = "http", address = "127.0.0.1:<ephemeral>", http = (), service = "gateway"),
  ],
);

const gatewayWorker :Workerd.Worker = (
  modules = [(name = "gateway.js", esModule = embed "gateway.js")],
  compatibilityDate = "2026-11-01",
  bindings = [
    (name = "LOADER",        workerLoader = (id = "plugins")),
    (name = "BRIDGE",        service = "bridge"),
    (name = "COMMERCE",      service = "commerce"),
    (name = "RUNTIME_TOKEN", text = "<random per generation>"),
  ],
);

const bridgeWorker :Workerd.Worker = (
  modules = [(name = "bridge.js", esModule = embed "bridge.js")],
  compatibilityDate = "2026-11-01",
  bindings = [(name = "UPSTREAM", service = "commerce")],
);
```

The gateway logic in outline:

```js
const CODE_API = "http://commerce/internal/plugins/v1/runtime/bundles";

export default {
  async fetch(request, env) {
    const { pluginId, digest, rest } = route(request); // /__plugin/{id}@{digest}/...
    const worker = env.LOADER.get(`${pluginId}@${digest}`, async () => {
      const res = await env.COMMERCE.fetch(`${CODE_API}/${pluginId}?digest=${digest}`, {
        headers: { authorization: `Bearer ${env.RUNTIME_TOKEN}` },
      });
      if (!res.ok) throw new Error("plugin bundle unavailable");
      const bundle = await res.json(); // { compatibilityDate, mainModule, modules }
      return {
        compatibilityDate: bundle.compatibilityDate,
        mainModule: bundle.mainModule,
        modules: bundle.modules, // { "index.js": "…", "chunk.js": "…" }
        env: { FASTCART_API: env.BRIDGE, FASTCART_PLUGIN_ID: pluginId },
        globalOutbound: null, // verified: fetch() throws
        // `limits` is accepted but not enforced in this build; see 4.4.
      };
    });
    return worker.getEntrypoint().fetch(rewrite(request, rest));
  },
};
```

This structure has four direct consequences:

1. **No per-plugin service and no per-plugin bridge.** Plugin identity is guaranteed by two mechanisms: `env.FASTCART_PLUGIN_ID` (structurally bound) and the per-request signed token (`sub` is the plugin id, and Rust checks that `sub` matches the path).
2. **Installing or removing a plugin is a database state change.** `LOADER.get` compiles lazily on first use; an upgrade produces a new digest and therefore a new loader id; the old instance is collected once nothing references it; disabling stops routing and may call the stub's `dispose()`.
3. **Code is delivered over an internal endpoint.** `GET /internal/plugins/v1/runtime/bundles/{id}?digest=…` returns the module map as JSON. Rust reads and caches it from the content-addressed artifact directory. This maps directly onto workerd's module semantics and avoids filesystem mounts.
4. **The cost is `--experimental`.** Without the flag, configuration loading fails outright. This must be documented as a deployment requirement and included in the workerd upgrade regression checklist.

Generation constraints:

- **The loader id must include the digest** (`{plugin}@{digest}`) so that upgrades take effect immediately and rollback does not hit a stale instance.
- **The module list** is collected recursively from the artifact, with the entry module first. Symlinks, traversal, and module sets above 64 MiB are rejected.
- **The compatibility date** comes from the manifest but is clamped to the range the host supports (`min_supported..=max_supported`).
- **Static assets never go through workerd.** All UI and storefront assets are served by Rust, following the `/_fastcart/assets/*` pattern with traversal checks and immutable cache headers.

## 4.2 The four request paths

### 4.2.1 Admin: UI → Rust → gateway → plugin (RPC)

```text
admin SPA                     Rust                                   workerd
   │  POST /api/v1/admin/plugins/{id}/rpc
   │  Cookie(session) + X-CSRF-Token
   ├──────────────────────────▶│ 1. session + CSRF checks
   │                           │ 2. RBAC: user permissions ∩ declared scopes
   │                           │ 3. mint a 30s context token
   │                           │ 4. forward to /__plugin/{id}@{digest}/admin/{method}
   │                           ├───────────────────────────────────────▶ gateway
   │                           │                                        ├─ check RUNTIME_TOKEN
   │                           │                                        └─ LOADER.get → plugin
   │                           │◀───────────────────────────────────────┤ JSON (≤ 512 KB)
   │◀──────────────────────────┤
```

Rules:

- The plugin never receives the admin session cookie.
- The context token carries `actor = { kind: "admin", id, permissions }`. Later calls made by the plugin are checked against the intersection of plugin scopes and the acting user's permissions, so a permission revocation takes effect immediately.
- Responses are size-capped; a timeout (default 10 s) returns 504 and writes an audit record.

### 4.2.2 Storefront: browser → Rust → gateway → plugin (HTTP)

```text
GET /apps/chat/api/conversations/42
  → Rust resolves the store from Host
  → verifies the plugin is enabled for the store and the path is not reserved
  → injects trusted headers: x-fastcart-store-id / locale / currency / request-id
                            x-fastcart-customer-token (short-lived, customer id if signed in)
                            x-fastcart-context-token (plugin API token)
  → forwards to /__plugin/chat@{digest}/fetch
  → plugin returns JSON, a stream, or a 101 WebSocket upgrade (verified)
```

Browser-facing requests never carry an internal service token. If a plugin needs business data, it uses the context token through the bridge.

### 4.2.3 Events: Rust outbox → delivery jobs → gateway → plugin

```text
business write → outbox(event_type, payload) → dispatcher
  → look up subscriptions for the store (plugin, event_type)
  → enqueue plugin_delivery (idempotency key = event_id + plugin_id)
  → job execution: POST /__plugin/{id}@{digest}/events { event, attempt }
  → 2xx marks done; 5xx/timeout retries with exponential backoff; N failures → dead letter
```

Semantics are **at-least-once**. The SDK exposes `event.id`; plugins must be idempotent.

### 4.2.4 Jobs: scheduled or plugin-enqueued

The manifest can declare jobs:

```json
"jobs": [
  { "name": "sync-orders", "schedule": "*/5 * * * *" },
  { "name": "send-digest", "schedule": null }
]
```

The host job queue stores `plugin_jobs` rows and invokes `/__plugin/{id}@{digest}/jobs/{name}`. `schedule` supports a five-field cron subset with a one-minute minimum, scanned by the maintenance loop.

## 4.3 Lifecycle

```text
install / enable / upgrade / disable
  → validate manifest and artifact
  → write installation state (store DB)
  → bump the store's plugin generation counter
  → gateway resolves the new {id}@{digest} on the next request and loads it lazily
  → old digest becomes unreachable; its loader entry is collected when unreferenced
```

- **No process restart.** The only cases that use the "start a new generation, drain the old one" pattern are workerd binary upgrades and changes to the loader/bridge modules themselves.
- **Crash handling:** the supervisor restarts workerd with backoff (1 s, 2 s, 4 s; at most 5 attempts). While it is down, plugin routes return 503, the admin shows "plugin runtime unavailable", and the storefront degrades silently (the boot script finds no manifest and mounts nothing).
- **Disabled runtime:** when `plugins.enabled` is false, install, upgrade, and enable return 409; listing, settings, disable, rollback, and delete remain available so operators can prepare and clean up; the internal bundle endpoint mounts when the runtime is enabled; the gateway token is taken from configuration or generated per process.
- **Lazy start:** workerd is not started until the first plugin is enabled. An installation without plugins pays no 85 MB.
- **Shutdown:** `fastcart-serve` drains traffic, then sends SIGTERM, then force-kills after a deadline (reusing the existing `fastcart-ssr-workerd` process logic).

## 4.4 Limits and isolation

| Resource      | Control point                                                                                                                                                    | Status                                                                                                                                                                     |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Request time  | Confirmed: admin 10 s, storefront 30 s, events 30 s, jobs 60 s                                                                                                   | Enforced in Rust; deadline + 5 s grace triggers the watchdog                                                                                                               |
| Concurrency   | Per-plugin semaphore (default 8), queue or 429                                                                                                                   | Implementable                                                                                                                                                              |
| Response size | 512 KB (RPC), 8 MB (storefront)                                                                                                                                  | Implementable                                                                                                                                                              |
| Egress        | Confirmed: 1,000 requests/day, 100 MB/day, up to 10 allowlisted hosts per plugin                                                                                 | Enforced by the egress proxy                                                                                                                                               |
| KV capacity   | Per-plugin, per-store quota (default 10 MB / 100k keys)                                                                                                          | Implementable                                                                                                                                                              |
| Logs          | Ring buffer plus sampling                                                                                                                                        | Implementable                                                                                                                                                              |
| CPU           | **Not enforced** (P0 V10–V13): `cpuMs` did not interrupt a 300 ms sync loop, entrypoint limits had no effect, and eight subrequests passed with `subRequests: 5` | Host deadline + **process-restart watchdog**, per-plugin semaphore, and a bridge-side subrequest counter. A synchronous loop can only be stopped by restarting the process |
| Memory        | Unenforced by the loader; process-level `MemoryMax` / `ulimit -v`                                                                                                | P0 measured **≈1.75–2 MB per loaded plugin**                                                                                                                               |
| JS heap       | Not measured by P0; depends on the workerd build                                                                                                                 | To verify in P1 hardening                                                                                                                                                  |

**Watchdog semantics (approved 2026-11-10):** when a request exceeds its deadline, Rust cancels its upstream call and returns 504. If the plugin worker still has not yielded 5 s after the deadline, the runtime marks the generation unhealthy and restarts the plugin host process. All in-flight plugin requests fail at that point, and the restart is recorded as `fastcart_plugin_host_restarts_total{reason="watchdog"}` plus an audit event. This is the only reliable stop for a synchronous loop in v1.

> **The loader's `limits` cannot be relied on.** The pinned build accepts `WorkerCode.limits = { cpuMs, subRequests }`, but a 300 ms synchronous loop, eight consecutive subrequests, and entrypoint-level limits were all completed without interruption. Host-side enforcement is mandatory.

**Isolation in v1:** one shared workerd process, one loader, one V8 isolate per plugin, one shared bridge. This is the boundary for operator-installed and reviewed plugins. Per-plugin process isolation is deferred (see [Roadmap](#roadmap-deferred-work)) and must not be assumed by the runtime API.

It must be stated explicitly in operator documentation that **a workerd isolate is not an OS security boundary**, consistent with ADR 0007.

---

# 5. Package and Manifest

## 5.1 Layout

```text
chat-widget/
├── plugin.json                 # manifest, the only entry point
├── worker/
│   └── index.js                # ES module, default export { fetch, admin?, events?, jobs? }
├── ui/                         # admin iframe pages (static)
│   ├── settings.html
│   ├── order-sidebar.html
│   └── assets/…
├── storefront/
│   ├── boot.js                 # custom element / mount logic
│   └── boot.css
└── assets/                     # icons and other static files
```

The package is a `tar.gz`. On upload the host verifies:

- total size ≤ 20 MB, ≤ 2000 files, each file ≤ 5 MB;
- no symlinks, no `..`, no absolute paths;
- `plugin.json` matches the JSON Schema;
- `entry.path` exists and ends in `.js` / `.mjs`;
- every `ui/` and `storefront/` reference exists;
- digest = SHA-256 over the canonicalized package content.

## 5.2 `plugin.json`

```json
{
  "schema_version": "1",
  "id": "chat-widget",
  "name": "Live Chat",
  "version": "1.2.0",
  "description": "Storefront chat bubble with an admin inbox.",
  "author": { "name": "Example Inc.", "url": "https://example.com" },
  "api_version": 2,

  "runtime": {
    "engine": "workerd",
    "compatibility_date": "2026-11-01",
    "compatibility_flags": ["nodejs_compat"]
  },
  "entry": { "type": "worker", "path": "worker/index.js", "export": "default" },

  "capabilities": {
    "data": ["catalog.read", "orders.read", "customers.read", "content.read"],
    "storage": ["kv.read", "kv.write"],
    "events": ["order.placed", "customer.created"],
    "network": ["api.openai.com", "api.example.com"],
    "admin": ["plugins.settings"],
    "storefront": ["store.read", "customer.read"]
  },

  "extensions": {
    "admin": [
      {
        "target": "admin.nav.section",
        "label": "Live Chat",
        "icon": "message-circle",
        "route": "/apps/chat-widget"
      },
      {
        "target": "admin.page.settings.tabs",
        "title": "Live Chat",
        "entry": "ui/settings.html",
        "height": 520
      },
      {
        "target": "admin.page.orders.details.side",
        "title": "Conversations",
        "entry": "ui/order-sidebar.html",
        "height": 240
      }
    ],
    "storefront": [
      {
        "kind": "embed",
        "placement": "body:end",
        "entry": "storefront/boot.js",
        "css": "storefront/boot.css",
        "pages": ["*"],
        "height_hint": 96
      }
    ]
  },

  "settings": {
    "schema": {
      "type": "object",
      "properties": {
        "accent": { "type": "string", "format": "color", "default": "#2563eb" },
        "greeting": { "type": "string", "default": "Hi! How can we help?" },
        "provider": { "type": "string", "enum": ["internal", "openai"], "default": "internal" }
      },
      "additionalProperties": false
    },
    "ui": "ui/settings.html"
  },

  "jobs": [{ "name": "sync-conversations", "schedule": "*/5 * * * *" }],

  "signatures": [],

  "build": { "build_id": "2026-11-10T10:00:00Z", "created_at": "2026-11-10T10:00:00Z" }
}
```

Notes:

- `capabilities` is the only authorization declaration. It is shown to the merchant at install time; an upgrade that adds scopes requires re-confirmation.
- `api_version` aligns with the existing constant in `crates/plugin`. The current value is `1`, but nothing depends on its shape yet. Define v2 semantics and keep v1 parse-only.
- `settings.schema` supports two UIs: a host-rendered form (simple cases) or a custom iframe via `settings.ui` (complex cases).
- `extensions.admin[].target` must exist in the host registry. Unknown targets are rejected at install time rather than silently never rendering.
- `signatures` is reserved and ignored in v1 (decision 3).

## 5.3 Validation rules

| Rule                                                                        | Rejection reason            |
| --------------------------------------------------------------------------- | --------------------------- |
| Identifier pattern `^[a-z0-9]([a-z0-9._-]*[a-z0-9])?$`, ≤ 120 chars         | Invalid plugin id           |
| Semver version (`x.y.z`)                                                    | Invalid version             |
| `api_version` in the supported set                                          | Unsupported host API        |
| `compatibility_date` within the host range                                  | Unsupported runtime version |
| Every declared target exists                                                | Unknown admin target        |
| Every scope is known to the host                                            | Unknown capability          |
| `network` entries are valid hostnames, no schemes, no ports, no IP literals | Invalid egress target       |
| Module set ≤ 64 MiB, total package ≤ 20 MB                                  | Artifact too large          |
| No duplicate targets within a plugin                                        | Duplicate target            |

## 5.4 Build contract (third-party JS/TS authors)

Because the authors are JavaScript/TypeScript developers, the build experience is part of the product.

| Item                | Rule                                                                                                                            | Reason                                                              |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Language            | TypeScript / JavaScript, ES modules                                                                                             | workerd executes ES modules and Wasm; there is no CommonJS runtime  |
| Bundling            | **Authors bundle** with Vite, esbuild, or wrangler; the artifact is a module graph                                              | No `node_modules` and no bare-specifier resolution at runtime       |
| Dependencies        | npm dependencies must be bundled into the artifact; no runtime `require` or bare imports                                        | workerd has no package manager                                      |
| Entry               | Default export `{ fetch, admin?, events?, jobs? }`                                                                              | Matches the Fetch Handler contract and stays portable to Cloudflare |
| Compatibility date  | Declared in the manifest, clamped by the host                                                                                   | Prevents relying on unverified runtime behavior                     |
| Compatibility flags | `nodejs_compat` is declared for clarity, but built-ins are available regardless (P0 V30)                                        | The flag is not a capability gate; bindings are                     |
| Node APIs           | Observed behavior, not a gate: `fs` is virtual, `net` is blocked by the outbound policy, `child_process` is unimplemented (5.4) | The boundary comes from bindings, not module availability           |
| Size                | Package ≤ 20 MB, module graph ≤ 64 MiB                                                                                          | Compile time and memory                                             |
| Source maps         | Shipped with the artifact and used by `plugin logs` to resolve stacks                                                           | Debuggability                                                       |
| Multiple entries    | `worker/`, `ui/`, and `storefront/` are bundled independently                                                                   | The three surfaces can use different stacks                         |

`nodejs_compat` policy (P0 observed behavior in the pinned build):

- **Built-ins are always available.** Without the `nodejs_compat` flag, `Buffer`, `process`, `node:path`, `node:util`, `node:crypto`, `node:stream`, and `node:events` all worked (P0 V30). Treat the flag as documentation, not a capability gate.
- **`node:fs` is a virtual filesystem.** It imports and exposes `bundle`, `tmp`, and `dev`; reading a host path such as `/etc/hostname` fails with ENOENT, and writes land in the virtual FS. There is no host filesystem access, but `/tmp` is per-isolate scratch, not durable storage.
- **`node:net` / `node:tls` / `node:dns` import, but connections are blocked** by `globalOutbound: null` (P0 V28: `net.connect` rejected with the outbound-policy error).
- **`node:child_process` imports, but `execSync` / `spawnSync` throw `ERR_METHOD_NOT_IMPLEMENTED`.**
- **`node:worker_threads` imports; worker creation is not yet characterized** and is on the P1 hardening list.
- **`process.env` exposes text bindings** (`FASTCART_PLUGIN_ID` was visible). Never pass secrets as text bindings; use per-request tokens and the secrets store instead.
- **Single-module shape limit (confirmed):** an 8 MB graph across eight modules loaded in 120–178 ms, while the same 8 MB in one module failed at load with a V8 stack overflow. Require multi-module bundles and enforce a **4 MB per-module cap** in preflight. `worker_threads` behavior is verified in P1 hardening.

Tooling:

- `@fastcart/plugin-cli`: `init` / `dev` / `pack`, esbuild under the hood. `dev` starts a Vite dev server for admin UI HMR and hot-swaps worker changes by loading a new digest.
- `@fastcart/plugin-sdk`: typed `ctx` (`api`, `kv`, `jobs`, `log`, `egress`) and `definePlugin()`.
- `@fastcart/plugin-ui`: `postMessage` client, `usePluginContext()`, theme tokens.
- `@fastcart/storefront-plugin-sdk`: custom element base class, Shadow DOM helpers, `FastCartExtensions.api()`.
- Templates: `chat` (worker + storefront + admin), `admin-only`, `webhook-sync`.

> The runtime never needs Node. Bundling happens on the author's machine or in CI; the host installs an already-bundled module graph.

## 5.5 Versioning and rollback

- `version` is semantic and used for display and upgrade policy. The immutable identity is the **digest**.
- `api_version` gates host compatibility; `min_host_api` expresses the minimum host capability a plugin needs.
- Installation records store `{ plugin_id, digest, manifest_json }`; multiple digests may coexist for rollback, with a cleanup policy consistent with the SSR artifact GC.
- Rollback switches the active digest. Because loader ids include the digest, the old instance is simply reactivated.

---

# 6. Extension Points

## 6.1 Admin UI targets (v1)

Naming convention: `admin.<page>.<location>`. The v1 set is deliberately small; targets are added in later releases without breaking the plugin contract.

### Enabled in v1

| Target                              | Placement                        | Context (props)  | Required permission |
| ----------------------------------- | -------------------------------- | ---------------- | ------------------- |
| `admin.nav.section`                 | Left navigation (append entries) | `{}`             | `plugins.read`      |
| `admin.page.overview.after-header`  | Overview page, after the header  | `{ period }`     | `plugins.read`      |
| `admin.page.orders.details.side`    | Order details, right column      | `{ orderId }`    | `orders.read`       |
| `admin.page.products.details.side`  | Product details, right column    | `{ productId }`  | `products.read`     |
| `admin.page.customers.details.side` | Customer details, right column   | `{ customerId }` | `customers.read`    |
| `admin.page.settings.tabs`          | Settings page, new tab           | `{}`             | `settings.read`     |

### Reserved (rejected in v1)

`admin.global.banner`, `admin.page.orders.list.after-table`, `admin.page.orders.details.after-summary`, `admin.page.orders.details.actions`, `admin.page.products.list.after-table`, `admin.page.products.details.tabs`, `admin.page.customers.list.after-table`, `admin.page.inventory.after-table`, `admin.page.promotions.list.after-table`, `admin.page.settings.after-sections`, `admin.page.themes.details.side`, `admin.command-palette.items`.

A manifest that declares a reserved target is rejected at install time with "target not enabled in this host version". Enabling a reserved target is a host-side release change; it does not change the plugin manifest format.

Constraints:

- Multiple plugins may target the same slot. The host sorts by `priority` and renders at most **three** instances per slot in v1.
- Detail-page targets receive **ids only**, never full objects. A plugin that needs data calls the API, which applies permission checks naturally.
- Every slot checks `plugins.read` for the current user. If a target's declared permission is not granted, it renders nothing.
- Target names are part of the host API version. Adding a target is backward compatible; removing one requires a new major host API.

## 6.2 Storefront extension points

| Kind     | Declaration                                                           | Use case                              | Injection                                              | v1                                                                         |
| -------- | --------------------------------------------------------------------- | ------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------- |
| `embed`  | `placement: "head:start" \| "head:end" \| "body:start" \| "body:end"` | Chat bubble, analytics, cookie banner | Gateway injects `<link>` and `<script type="module">`  | ✅                                                                         |
| `slot`   | `slot: "product.details.after"`                                       | Product reviews, cart extras          | Theme outputs a container, boot runtime mounts into it | ⏸ deferred (markers exist; per-template containers are the remaining work) |
| `block`  | `block: "reviews"`                                                    | Merchant-positioned block             | Page editor integration                                | ⏸ deferred                                                                 |
| `server` | `render: "/storefront/render/{slot}"`                                 | SEO-friendly server-rendered block    | Rust requests HTML during injection                    | ⏸ deferred                                                                 |

**Trade-off:** `embed` works with any theme because it is a response transform. `slot` is more precise but requires a container in each template; the marker protocol now exists, so slots only need per-template containers and can be added without a contract change.

**v1 is read-only for business data.** Write scopes such as `catalog.write` and `orders.write` are reserved and rejected by the host. Plugin-owned state is KV, settings, and secrets.

## 6.3 Backend extension points

| Capability                         | Manifest declaration                  | Host behavior                                   | Phase    |
| ---------------------------------- | ------------------------------------- | ----------------------------------------------- | -------- |
| Public routes                      | Always available under `/apps/{id}/*` | Forwarded to the plugin's `fetch`               | P2       |
| Internal API                       | `capabilities.data`                   | Bridge forwards to `/internal/plugins/v1/*`     | P2       |
| KV                                 | `capabilities.storage`                | Host storage, quotas, per-store isolation       | P2       |
| Events                             | `capabilities.events`                 | outbox → delivery job → `events` export         | P5       |
| Jobs                               | `jobs[]`                              | Host queue → `jobs/{name}` export               | P5       |
| Egress                             | `capabilities.network`                | Egress proxy with allowlist and SSRF protection | P2       |
| Providers (payment/shipping/email) | `providers[]`                         | Host calls into the plugin                      | Deferred |
| Pure functions (discounts/pricing) | `functions[]` (Wasm)                  | Host calls on the hot path                      | Deferred |

---

# 7. Admin UI Runtime

## 7.1 Host side: slot registry

New code under `apps/admin/src`:

```text
src/plugins/
├── targets.ts          # target constants, props types per target
├── registry.tsx        # target → container style (card, sidebar, tab, button)
├── runtime.ts          # fetch the extension manifest for the current store (cached, invalidated)
├── plugin-slot.tsx     # <PluginSlot target="..." context={...} />
├── plugin-frame.tsx    # iframe loading + postMessage lifecycle
└── plugin-page.tsx     # full-page plugin (/apps/$pluginId/$)
```

Usage (example: product details page):

```tsx
<PluginSlot target="admin.page.products.details.side" context={{ productId }} />
```

`<PluginSlot>` responsibilities:

1. Filter the runtime manifest for the target, for plugins enabled in this store, and for slots the current user may see.
2. Lazily render a `<PluginFrame>` per instance (`IntersectionObserver`, load only when visible).
3. Provide an error boundary: a crashing plugin affects only its own card and shows a placeholder.
4. Render nothing when the list is empty (no layout shift).

Navigation and full pages:

- `admin.nav.section` entries merge into the existing `NAV_GROUPS` read-only.
- Full pages use **one** catch-all route `/apps/$pluginId/$`, resolved against the manifest, so no dynamic route registration is needed.

## 7.2 Plugin side: iframe sandbox

```html
<!-- GET /_fastcart/plugin-ui/{plugin_id}/{digest}/settings.html?t=<mount_token> -->
<iframe sandbox="allow-scripts allow-forms" referrerpolicy="no-referrer" src="…"></iframe>
```

Response headers:

```http
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline';
  img-src 'self' data: https:; font-src 'self'; connect-src 'none'; frame-ancestors 'self'
X-Content-Type-Options: nosniff
Cache-Control: no-store            # HTML entry; assets/* are immutable
```

- `sandbox` omits `allow-same-origin`, giving the frame an opaque origin: it cannot read admin cookies or `localStorage`.
- `connect-src 'none'` forbids direct network access from plugin UI; all data flows through host RPC, which is auditable and permission-checked.
- Asset URLs include the digest and are served with `Cache-Control: public, max-age=31536000, immutable`.

## 7.3 `postMessage` protocol (v1)

Every message carries `v`, `type`, and `channel`. The channel is generated per mount and delivered only in `init`.

| Direction    | Type             | Payload                                                                                                     | Purpose                              |
| ------------ | ---------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| host → frame | `init`           | `{target, pluginId, store, locale, theme:{mode,tokens}, context, user:{id,name,permissions}, capabilities}` | Sent immediately after load          |
| host → frame | `context:update` | `{context}`                                                                                                 | Page data changed                    |
| host → frame | `theme:update`   | `{mode,tokens}`                                                                                             | Light/dark switch                    |
| frame → host | `ready`          | `{}`                                                                                                        | Request `init`                       |
| frame → host | `rpc`            | `{id, method, params}`                                                                                      | Call a host capability               |
| host → frame | `rpc:result`     | `{id, ok, result?, error?}`                                                                                 | Result                               |
| frame → host | `resize`         | `{height}`                                                                                                  | Clamped to `[min,max]`               |
| frame → host | `navigate`       | `{to}`                                                                                                      | Site-relative paths only             |
| frame → host | `toast`          | `{level, message}`                                                                                          | Host notification                    |
| frame → host | `open:external`  | `{url}`                                                                                                     | Confirmed by the host before opening |

Security rules:

- The host accepts a message only when `event.source === iframe.contentWindow`. An opaque origin cannot be checked by origin, so source plus channel is the check.
- The channel is regenerated on every mount and invalidated on unmount.
- `rpc.method` must be declared in the manifest; undeclared methods are rejected and logged.
- Payload limit 256 KB; `resize` is throttled to one message per 100 ms.

## 7.4 RPC to host capabilities

```text
frame → postMessage({type:"rpc", method:"orders.get", params:{id}})
host  → validate method ∈ manifest.rpc
      → POST /api/v1/admin/plugins/{plugin}/rpc
         { method, params, target, mountToken }
Rust  → session + CSRF + (user permissions ∩ plugin scopes)
      → gateway /__plugin/{id}@{digest}/admin/{method}
      → JSON response
host  → postMessage({type:"rpc:result", ...})
```

v1 uses a **declarative allowlist** of methods, never arbitrary HTTP passthrough. This keeps "what a plugin can do" visible at install time.

## 7.5 Plugin management UI

New page `apps/admin/src/pages/plugins.tsx` (or a Plugins tab under Settings):

- **List:** installed plugins (version, status, last error, request rate) and available plugins (uploaded artifacts).
- **Install:** upload `tar.gz` → show the manifest summary and **permission consent** → choose store scope → enable.
- **Details:** configuration form (JSON Schema-generated or `settings.ui`), secrets, logs, metrics, rollback, uninstall.
- **Developer mode:** show "loading from a local directory" and offer one-click reload.

Permissions: add `plugins.read`, `plugins.write`, `plugins.install` (platform migration), granted to owner/admin roles by default.

---

# 8. Storefront UI Runtime

## 8.1 Theme markers and injection strategy

A **marker** is an explicit placeholder inside theme HTML that tells the host where extension tags may be written. It is a contract between the theme and the FastCart gateway, not a plugin-facing API: plugins declare a placement (`head:end`, `body:end`), and the theme declares where the host is allowed to write.

### 8.1.1 Marker forms

Two equivalent forms are recognized. The comment form is canonical for plain HTML templates; the script form exists because JSX/TSX cannot emit comments through normal children.

```html
<!-- comment form (plain HTML, Astro, generic worker templates) -->
<!--fastcart:extensions-head-->
<!--fastcart:extensions-body-->

<!-- script form (React/JSX/TanStack/Svelte templates) -->
<script type="application/fastcart-extensions" data-slot="head"></script>
<script type="application/fastcart-extensions" data-slot="body"></script>
```

Rules:

- Tokens are exact and case-sensitive. The comment must contain no extra whitespace; the script marker must keep `type="application/fastcart-extensions"` and `data-slot`.
- The first occurrence of a slot's marker in the document wins. Duplicates are replaced once and logged as a theme warning.
- A marker is removed from the output and replaced by the host-generated tags.
- Markers inside `<template>` or inside an HTML comment that the theme itself commented out cannot be distinguished and will not be injected; theme authors must not wrap markers in other comments.
- The scanner is byte-oriented and does not parse script or style content. The token is long and namespaced, so an accidental match is unlikely; a theme must not print the token inside a JS or CSS string. If that happens, the theme warning for duplicate or misplaced markers applies.

### 8.1.2 Why markers

| Problem                                   | Fallback (`</body>` rewrite)         | Marker                                          |
| ----------------------------------------- | ------------------------------------ | ----------------------------------------------- |
| Inline JS contains the string `"</body>"` | May inject in the wrong place        | Unique token, no false positives                |
| Streamed response; `</body>` appears late | Must buffer a 32 KB tail             | Injection happens as soon as the marker is seen |
| Theme omits the closing tag               | Misses injection                     | Still works                                     |
| Styles must go into `<head>`              | Needs a second scanner for `</head>` | Head marker is explicit                         |
| Inject only on some templates             | Not possible without path rules      | Put the marker only in those templates          |
| Error pages or HTML fragments             | Needs heuristics to avoid injecting  | No marker, no injection                         |

### 8.1.3 Selection and fallback

1. Only `text/html`, status 200, storefront SSR path, and only when the store has at least one enabled storefront extension.
2. Only for full documents: the first 1 KB must contain `<!doctype` or `<html`. Fragments and error bodies are skipped.
3. **Marker first.** If a slot's marker is present, the host injects there and does not use the fallback for that slot.
4. **Fallback otherwise.** If no marker is found, the host injects before `</head>` / `</body>`, holding back a 32 KB tail to find the last closing tag.
5. **Strict mode.** A theme artifact may declare `extensions.markers` in its manifest. In strict mode the gateway skips the tail buffer entirely; a missing marker means no injection for that slot and a warning, and the theme's smoke test is expected to fail at build time.
6. If neither the marker nor the closing tag is found, the response is flushed unchanged and a debug log is written. Injection never fails a response.

### 8.1.4 Streaming algorithm

```text
for each chunk from the SSR worker:
  append to a rolling scan window (max marker length + 1)
  if a head or body marker is found:
      emit bytes before it
      emit the host tags for that slot
      drop the marker
      continue streaming with marker mode for that slot
  else if fallback mode is enabled:
      keep at most the last 32 KB in the tail buffer
      emit everything older
during streaming:
  if the head slot is unresolved and fallback mode is on and </head> is seen:
      inject head tags immediately; resolve head
at EOF:
  if the body slot is still unresolved and fallback mode is on:
      find the last </body> in the tail; inject; flush
  else:
      flush the tail unchanged
  if the buffer contains the injection sentinel:
      flush unchanged (idempotent)
```

- The scan window is bounded by the longest marker (about 60 bytes), so marker detection costs constant memory.
- The head fallback is decided as soon as `</head>` is seen; only the body fallback needs the 32 KB tail. P0 confirmed that waiting for EOF would push the head beyond the tail on large pages.
- **Idempotency uses a sentinel.** Every injection emits `<!--fastcart:extensions-injected-->` once; if a response already contains it, injection is a no-op. P0 V38 verified the second pass is byte-identical.
- `content-length` is removed because the body length changes; the response becomes chunked.
- If the response carries `Content-Encoding` other than identity, injection is skipped. The gateway requests `Accept-Encoding: identity` from the SSR worker to make this rare.
- **Injection happens after cache lookup.** The SSR cache stores the theme's original HTML, markers included; the extension manifest is cached in memory per store and invalidated by generation. Enabling or disabling a plugin never poisons the page cache.

### 8.1.5 Theme template changes

The default templates (`templates/storefront-tanstack`, `templates/storefront-astro`, `templates/storefront-sveltekit`, `templates/storefront-generic`) and `apps/storefront` add both markers to their root layout. Because each framework emits markup differently:

- TanStack Start and other React-based templates use the script form in the root route (`<script type="application/fastcart-extensions" data-slot="head" />` inside `<head>`, and the body equivalent at the end of `<body>`).
- Astro and the generic Fetch-handler template use the comment form, which survives layout composition.
- The theme artifact manifest gains an optional field:

```json
"extensions": { "markers": ["head", "body"] }
```

When present, the gateway trusts the declaration and skips tail buffering. When absent, markers are still honored, but the fallback stays enabled.

### 8.1.6 Security notes

- Injected tags are generated entirely by the host from validated manifest fields (digest-scoped entry URLs). No plugin-controlled HTML is written at a marker position.
- If the response carries a CSP with a nonce, the injected script reuses that nonce. The host never weakens an existing policy.
- Marker replacement is textual and idempotent: a second pass over the same HTML finds no marker and makes no change.

## 8.2 Injected content

```html
<!-- fastcart:extensions-injected -->
<!-- once per response, idempotency sentinel -->

<!-- head -->
<link rel="stylesheet" href="/_fastcart/extensions/chat-widget/ab12…/boot.css" media="all" />

<!-- end of body -->
<script>
  window.__FASTCART_EXTENSIONS__ = {
    v: 1,
    store: "store_123",
    items: [
      {
        id: "chat-widget",
        digest: "ab12…",
        kind: "embed",
        placement: "body:end",
        entry: "/_fastcart/extensions/chat-widget/ab12…/boot.js",
      },
    ],
  };
</script>
<script type="module" src="/_fastcart/extensions/runtime.js" nonce="…" async></script>
```

- The manifest is inlined to avoid a round trip; it is capped at 8 KB and falls back to an external `manifest.json` above that.
- Non-HTML responses, non-200 responses, and encoded responses are skipped.
- If a CSP with a nonce is present, the same nonce is used. Otherwise the host adds `script-src 'self'` only when it already controls the CSP; it never weakens an existing policy.
- Disabling a plugin removes it from the manifest; already-open pages keep the old behavior until the next navigation.
- The sentinel at the top of the injected block makes the transform idempotent; a second pass returns the input unchanged (P0 V38).

## 8.3 Boot runtime and custom elements

```js
// /_fastcart/extensions/runtime.js (host-provided, versioned)
(() => {
  const cfg = window.__FASTCART_EXTENSIONS__;
  if (!cfg) return;
  const mount = (item) => import(item.entry).catch(report);
  for (const item of cfg.items) mount(item);
  window.FastCartExtensions = {
    register(tag, define) {
      if (!customElements.get(tag)) customElements.define(tag, define);
    },
    config: (id) => cfg.items.find((i) => i.id === id),
    api: (path, init) => fetch(path, { credentials: "same-origin", ...init }),
  };
})();
```

Plugin side (chat bubble):

```js
// storefront/boot.js
class ChatBubble extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `<style>…</style><button part="bubble">…</button>`;
    root.querySelector("button").addEventListener("click", () => this.open());
  }
  async open() {
    const res = await window.FastCartExtensions.api("/apps/chat-widget/conversations", { method: "POST" });
    …
  }
}
customElements.define("fc-chat-bubble", ChatBubble);

const el = document.createElement("fc-chat-bubble");
el.style.cssText = "position:fixed;right:24px;bottom:24px;z-index:2147483000";
document.body.appendChild(el);
```

Conventions:

- Plugins must use **Shadow DOM** to avoid leaking styles; host theme tokens are exposed as CSS variables (`--fc-color-brand`, etc.).
- Maximum `z-index` is `2147483000` so checkout overlays stay on top. Plugins may declare `exclude_paths` (for example `/checkout/*`).
- Global keyboard or scroll interceptors require a `global-listeners` capability.
- Accessibility: `aria-label`, keyboard dismissal, and `prefers-reduced-motion` are mandatory.

## 8.4 Performance budget

| Item                               | Budget                                                       |
| ---------------------------------- | ------------------------------------------------------------ |
| Boot script (gzip)                 | ≤ 3 KB                                                       |
| Per-plugin first-load JS (gzip)    | ≤ 50 KB                                                      |
| First-load CSS                     | ≤ 20 KB                                                      |
| Extra requests caused by injection | ≤ 3 (runtime + inline manifest + plugin boot)                |
| Main-thread impact                 | Must not block before `DOMContentLoaded`; `async` by default |
| Overlay initialization             | ≤ 100 ms excluding network                                   |

Plugins exceeding the budget receive a warning on the install page; installation is not blocked.

---

# 9. Plugin ↔ Platform API

## 9.1 Namespaces

| Namespace                 | Caller                         | Authentication                                    |
| ------------------------- | ------------------------------ | ------------------------------------------------- |
| `/apps/{plugin_id}/*`     | Browser / third-party webhooks | Plugin-owned (trusted store headers are injected) |
| `/internal/plugins/v1/*`  | Plugin, through the bridge     | Short-lived signed token                          |
| `/api/v1/admin/plugins/*` | Admin SPA                      | Session + CSRF + RBAC                             |
| `/_fastcart/plugin-ui/*`  | iframe                         | Mount token (HTML) / public immutable assets      |
| `/_fastcart/extensions/*` | Storefront                     | Public (same origin)                              |

## 9.2 Tokens

```jsonc
{
  "v": 1,
  "iss": "fastcart",
  "sub": "chat-widget", // plugin id
  "aud": "plugin-api",
  "store": "store_123", // the only store this token allows
  "scopes": ["catalog.read", "kv.write"],
  "actor": { "kind": "admin", "id": "user_1", "permissions": ["orders.read", "..."] },
  // or { "kind": "event", "id": "evt_…" } / { "kind": "job", "id": "job_…" }
  "jti": "…", // single use, replay protection
  "exp": 1799999999, // 30 seconds
}
```

- HMAC-SHA256 keyed by an installation key derived from `(store, plugin)`, rotated per generation.
- Rust validates: signature, `sub` matches the request path, `store` matches the installation, `jti` unused, not expired.
- **Intersection rule:** effective scopes = manifest scopes ∩ token scopes ∩ (for admin actors) the user's permissions.
- **Tokens are per request, never environment variables.** The gateway passes one in the trusted header `x-fastcart-context-token`; the SDK forwards it to the bridge. A plugin has no long-lived credential and cannot mint tokens for another store.
- **Background work without an actor** (cron jobs, self-initiated long tasks) cannot reuse a request token. Such work must go through host jobs, which mint a token with `actor.kind = "job"` at execution time.

> **P2 implementation note:** tokens are signed with one key derived from the deployment secret; `store` and `sub` are bound in the claims. `jti` is currently a correlation id rather than a single-use nonce, because one plugin request legitimately makes several bridge calls. Per-installation key derivation and replay protection are P6 hardening; see [plugin-p2-report.md](plugin-p2-report.md).

## 9.3 Capability → endpoint mapping

| Scope                  | Endpoint                                                                   | Notes                                               |
| ---------------------- | -------------------------------------------------------------------------- | --------------------------------------------------- |
| `core.context`         | `GET /internal/plugins/v1/context`                                         | Store, locale, currency, non-secret plugin settings |
| `catalog.read`         | `GET /internal/plugins/v1/products`, `/products/{id}`, `/collections`      | Read-only, page size ≤ 100                          |
| `orders.read`          | `GET /internal/plugins/v1/orders`, `/orders/{id}`                          | Redacted by default: no payment details             |
| `customers.read`       | `GET /internal/plugins/v1/customers/{id}`                                  | Email returned only with an extra scope             |
| `content.read`         | `GET /internal/plugins/v1/pages/{handle}`                                  |                                                     |
| `carts.read`           | `GET /internal/plugins/v1/carts/{id}`                                      | Storefront use case                                 |
| `kv.read` / `kv.write` | `GET/PUT/DELETE /internal/plugins/v1/kv/{ns}/{key}`, `POST /kv/{ns}:query` | Values ≤ 256 KB; keys prefixed with `plugin:`       |
| `events.subscribe`     | No endpoint; delivered by the host                                         |                                                     |
| `network.egress`       | `POST /internal/plugins/v1/egress`                                         | Body `{url, method, headers, body}`                 |
| `jobs.enqueue`         | `POST /internal/plugins/v1/jobs/{name}`                                    | Delay ≤ 30 days                                     |

Write operations require an `Idempotency-Key`, deduplicated by the host.

## 9.4 Egress proxy

```text
plugin → bridge → POST /internal/plugins/v1/egress {url:"https://api.openai.com/v1/..."}
Rust:
  1. Parse the URL; HTTPS only; hostname must match the manifest allowlist (supports *.example.com)
  2. Resolve DNS and reject private/loopback/link-local/metadata addresses (169.254.169.254 etc.);
     re-check at connect time to prevent DNS rebinding
  3. Strip Cookie/Authorization unless the manifest explicitly declares `forward_auth`
  4. Timeout 15 s, response ≤ 5 MB, daily byte and request quotas
  5. Record host, status, bytes, latency (never bodies)
```

This is the only egress path. The plugin worker's global `fetch` is blocked by `globalOutbound: null`.

## 9.5 Event and job semantics

- **At-least-once**, idempotent per `(plugin_id, event_id)`.
- Retries: exponential backoff from 5 s, five attempts, then dead letter; the admin can replay a dead letter.
- Event payloads are **snapshots** (for example, an order summary at placement time), not live references. A plugin that needs full data calls the API.
- Jobs: `jobs/{name}` receives `{id, attempt, scheduled_at, payload}`. A single execution times out after 60 s; jobs with the same name run serially.

## 9.6 TypeScript SDK

```ts
// @fastcart/plugin-sdk
import { definePlugin } from "@fastcart/plugin-sdk";

export default definePlugin({
  async fetch(ctx, request) {
    const url = new URL(request.url);
    if (url.pathname === "/api/conversations") {
      const order = await ctx.api.orders.get({ id: ctx.query("order_id") });
      await ctx.kv.set("last_order", order.id);
      return Response.json({ ok: true });
    }
    return new Response("not found", { status: 404 });
  },

  async admin(ctx, method, params) {
    if (method === "conversations.list") {
      return ctx.api.requests.get("/internal/plugins/v1/kv/chat/conversations");
    }
  },

  async events(ctx, event) {
    if (event.type === "order.placed") {
      await ctx.jobs.enqueue("sync-conversations", { orderId: event.data.id });
    }
  },
});
```

`ctx` exposes `store`, `locale`, `settings` (validated), `api` (typed methods), `kv`, `jobs`, `log`, and `egress`. Internally the SDK translates each call into `env.FASTCART_API.fetch()` with the current context token.

---

# 10. Data Model

## 10.1 Platform database (`migrations/platform/0002_plugin_permissions.sql`)

```sql
INSERT OR IGNORE INTO permissions (id, name, description) VALUES
  ('perm_plugins_read',    'plugins.read',    'View installed plugins'),
  ('perm_plugins_write',   'plugins.write',   'Configure and toggle plugins'),
  ('perm_plugins_install', 'plugins.install', 'Install, upgrade and remove plugins');
-- Granted to the owner/admin roles, consistent with existing role seeds.
```

## 10.2 Store database (`migrations/store/0048_plugins.sql`)

```sql
CREATE TABLE IF NOT EXISTS plugin_installations (
  id TEXT PRIMARY KEY,
  plugin_id TEXT NOT NULL,
  digest TEXT NOT NULL,
  version TEXT NOT NULL,
  manifest_json TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  settings_json TEXT NOT NULL DEFAULT '{}',
  settings_revision INTEGER NOT NULL DEFAULT 0,
  granted_scopes TEXT NOT NULL DEFAULT '[]',
  previous_digest TEXT,
  installed_by TEXT,
  installed_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (plugin_id)
);
CREATE INDEX IF NOT EXISTS idx_plugin_installations_enabled ON plugin_installations (enabled);

CREATE TABLE IF NOT EXISTS plugin_secrets (
  plugin_id TEXT NOT NULL,
  name TEXT NOT NULL,
  ciphertext BLOB NOT NULL,
  nonce BLOB NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, name)
);

CREATE TABLE IF NOT EXISTS plugin_kv (
  plugin_id TEXT NOT NULL,
  namespace TEXT NOT NULL,
  key TEXT NOT NULL,
  value BLOB NOT NULL,
  expires_at INTEGER,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_plugin_kv_expiry ON plugin_kv (expires_at);

CREATE TABLE IF NOT EXISTS plugin_event_subscriptions (
  plugin_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (plugin_id, event_type)
);

CREATE TABLE IF NOT EXISTS plugin_deliveries (
  id TEXT PRIMARY KEY,
  plugin_id TEXT NOT NULL,
  event_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  payload TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  available_at INTEGER NOT NULL,
  last_error TEXT,
  created_at INTEGER NOT NULL,
  UNIQUE (plugin_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_plugin_deliveries_claim ON plugin_deliveries (status, available_at);

CREATE TABLE IF NOT EXISTS plugin_jobs (
  id TEXT PRIMARY KEY,
  plugin_id TEXT NOT NULL,
  name TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  available_at INTEGER NOT NULL,
  last_error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_logs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  plugin_id TEXT NOT NULL,
  level TEXT NOT NULL,
  message TEXT NOT NULL,
  fields_json TEXT,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_plugin_logs_plugin ON plugin_logs (plugin_id, created_at);
```

- v1 does **not** add a generic `plugin_records` table. Session and message data lives in KV; the design is revisited after the chat pilot if query or index requirements appear.
- Logs are trimmed by count and age (default 7 days, 10k rows per plugin) by the maintenance loop.
- One installation row per plugin; rollback is a digest switch (`previous_digest`), and the old artifact remains on disk until GC.
- Every table lives in the store database, so isolation and deletion follow the existing store lifecycle.

## 10.3 Artifact storage

```text
<data_dir>/plugins/
├── artifacts/<digest>/            # immutable, content-addressed
│   ├── plugin.json
│   ├── worker/index.js
│   ├── ui/…
│   └── storefront/…
└── runtime/
    └── generations/<gen_id>/
        ├── config.capnp
        └── gateway.js, bridge.js
```

The same safety rules as the SSR artifact store apply: no symlinks, no `..`, and read-only files after activation. A maintenance sweep every 360 ticks (about one hour) collects digests referenced by every store's `plugin_installations` (current and rollback) and deletes unreferenced artifacts older than one hour, so an in-flight install is never raced.

---

# 11. Security and Threat Model

| #   | Threat                                            | Control                                                                                                                                                              | Residual risk                                                                                                                                                                                                                                |
| --- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| T1  | A plugin reads another store's data               | Tokens bind the store; all queries go through Rust services; no database handle                                                                                      | A plugin installed on multiple stores can read all of them (same as a Shopify app); mitigated by operator review                                                                                                                             |
| T2  | A plugin steals an admin cookie                   | Sandboxed iframe (opaque origin) + host-proxied RPC + no cookie forwarding to workers                                                                                | None                                                                                                                                                                                                                                         |
| T3  | A plugin calls arbitrary internal APIs            | The bridge hard-codes the path prefix; Rust validates scopes per endpoint                                                                                            | None (the bridge cannot be repointed by a plugin)                                                                                                                                                                                            |
| T4  | A plugin exfiltrates data                         | `globalOutbound: null` + egress allowlist + private-address blocking + quotas                                                                                        | Allowlisted hosts can still be abused (for example, a public paste service); review is required                                                                                                                                              |
| T5  | A plugin DoSes the runtime (loop, large response) | Request deadlines, concurrency semaphore, response caps, process-restart watchdog, OS quotas                                                                         | **Measured:** a 3 s synchronous loop delayed a healthy plugin by 2.9 s. There is no hard CPU kill in v1; the watchdog restarts the whole plugin host. Per-plugin process isolation is deferred; operators must install only reviewed plugins |
| T6  | A plugin XSSes the admin                          | iframe sandbox + CSP + no `allow-same-origin` + message validation                                                                                                   | Overlay/full-screen targets are intentionally not offered                                                                                                                                                                                    |
| T7  | A plugin injects malicious storefront script      | Same-origin hosting + CSP `script-src 'self'` + review                                                                                                               | Client code shares the page's privileges; this is inherent to app-embed-style extensions                                                                                                                                                     |
| T8  | A plugin forges identity                          | Tokens are minted by Rust; the bridge injects the plugin id; `jti` is single-use                                                                                     | Replay within 30 s is possible only by the plugin that received the token                                                                                                                                                                    |
| T9  | Supply-chain compromise (npm dependency)          | Immutable artifact + digest + install consent + manual review                                                                                                        | **No signature verification in v1** (decision 3); provenance is operator trust only                                                                                                                                                          |
| T10 | A plugin reads the server filesystem              | `node:fs` is a **virtual filesystem** (`bundle`, `tmp`, `dev`); host paths fail with ENOENT (P0 V28). Modules come only from the artifact; assets are served by Rust | Virtual `/tmp` is per-isolate scratch and not durable; no host disk binding exists. Any future disk binding must re-open this threat                                                                                                         |
| T11 | A plugin escapes the isolate                      | workerd is maintained upstream; still treated as a non-OS boundary                                                                                                   | No per-plugin process isolation in v1; container/VM would be required for hostile code                                                                                                                                                       |
| T12 | Plugins interfere with each other                 | No bindings between plugins; one isolate each                                                                                                                        | They share the process thread pool, so CPU contention remains                                                                                                                                                                                |

Additional rules:

- A workerd isolate is **not** an OS security boundary (consistent with ADR 0007). Operator documentation must say so.
- Plugin UI must use `connect-src 'none'`; all data flows through RPC.
- `nodejs_compat` is enabled with an allowlist (5.4). No disk or raw-socket binding may be added without revisiting this threat model.
- v1 accepts only operator-installed plugins. There is no marketplace, no signature check, and no revocation feed; the mitigation is install-time consent plus review.

## 11.1 Provenance in v1

| Source                           | Loader        | Controls                                                                           |
| -------------------------------- | ------------- | ---------------------------------------------------------------------------------- |
| First-party (FastCart official)  | Shared loader | Reviewed in-repo; full declared capability set                                     |
| Third-party (operator-installed) | Shared loader | Consent screen at install, capability diff on upgrade, per-plugin quota, audit log |

Deferred with signing: publisher identity, revocation lists, automatic trust promotion. The manifest already reserves `signatures[]`, and the loader can later reject by `plugin_id` or `digest` without an SDK change.

---

# 12. Observability and Operations

**Metrics** (Prometheus, matching existing naming):

```text
fastcart_plugin_requests_total{plugin,kind,status}        # kind: admin|storefront|event|job
fastcart_plugin_request_duration_seconds{plugin,kind}     # histogram
fastcart_plugin_runtime_up{generation}                    # gauge
fastcart_plugin_load_failures_total{plugin}
fastcart_plugin_kv_bytes{plugin}                          # periodic scan
fastcart_plugin_egress_bytes_total{plugin,host}
fastcart_plugin_jobs_failed_total{plugin,name}
fastcart_plugin_delivery_lag_seconds{plugin}
```

**Logs:**

- Every line carries `plugin_id`, `generation`, and `request_id`.
- `console.log` output is captured from workerd stdout; `ctx.log` is the recommended structured path.
- Secrets (tokens, cookies, emails) are redacted by the host before storage.

**Health:**

- `GET /health/plugins` → `{ status, generation, plugins: [{id, healthy, last_error}] }`.
- Per-plugin health comes from an optional `/__fastcart/health` export; otherwise the plugin is considered healthy when its last request succeeded.

**CLI:**

```bash
fastcart plugin list --store demo
fastcart plugin enable chat-widget --store demo
fastcart plugin logs chat-widget --store demo --follow
fastcart plugin rollback chat-widget --store demo
fastcart plugin doctor --store demo      # config, artifacts, tokens, ports, module graph
```

---

# 13. Performance and Capacity

Measured (section 0), plus estimates:

| Scenario                                               | Measured (P0)                                                                | Estimate            |
| ------------------------------------------------------ | ---------------------------------------------------------------------------- | ------------------- |
| workerd cold start                                     | 61.8 ms                                                                      | —                   |
| Plugin call latency (gateway + loader + bridge + host) | p50 0.31–0.45 ms / p95 0.50–0.59 ms                                          | —                   |
| 10 loaded plugins                                      | +17.5–17.6 MB (75.9–76.4 → 93.5–94.0 MB)                                     | ≈1.75 MB per plugin |
| 30 loaded plugins                                      | +59.5 MB (75.9 → 135.4 MB)                                                   | ≈1.98 MB per plugin |
| 50 loaded plugins                                      | —                                                                            | ~175–185 MB total   |
| 1 MB module graph compile                              | 19–28 ms                                                                     | —                   |
| 8 MB module graph compile                              | 120–178 ms                                                                   | —                   |
| HTML injection (1 MB document)                         | 1.35–1.61 ms                                                                 | —                   |
| Install, enable, or upgrade                            | A database write plus a lazy compile on the next request; no process restart | —                   |

Optimizations:

- Plugin traffic never touches the SSR group.
- The extension manifest is cached in Rust memory and invalidated by store generation, so admin page changes cost one request.
- The storefront runtime is immutably cached; the plugin manifest is inlined.
- Plugin UI is lazy-loaded through `IntersectionObserver`.
- Event delivery uses batched claims, like the existing job queue.

---

# 14. Developer Experience

```bash
# Scaffold
fastcart plugin init chat-widget --template chat

# Local development: temporary workerd, connected to a local fastcart-serve dev endpoint
fastcart plugin dev ./chat-widget --store demo --port 8790
#   - file changes → bundle → new digest → hot swap (no process restart)
#   - admin UI served by the plugin's own Vite dev server for HMR

# Package and install
fastcart plugin pack ./chat-widget -o chat-widget.tar.gz
fastcart plugin install chat-widget.tar.gz --store demo
```

- `dev` uses a store-scoped development token that is never persisted, and shows a "development" banner in the admin.
- Templates provide `worker/index.js`, `ui/settings.html` (with `@fastcart/plugin-ui`), `storefront/boot.js`, `plugin.json`, `tsconfig`, and Vitest.
- `@fastcart/plugin-sdk` exposes typed `ctx.api.*`; `@fastcart/plugin-ui` provides the `postMessage` client and theme tokens; `@fastcart/storefront-plugin-sdk` provides the custom element base class.
- Testing: `createTestRuntime()` simulates `ctx`, KV, and events in Node without starting workerd.

---

# 15. Implementation Plan

| Phase                                   | Scope                                                                                                                                                                                            | Deliverables                                                                                                                                                                                                                     | Acceptance criteria                                                                                                                                 | Effort |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| **P0** Contract validation ✅           | Worker Loader (`--experimental`), `env` service binding injection, `globalOutbound: null`, `limits` behavior, WebSocket/streaming, module-size cap, concurrency and GC, `nodejs_compat` behavior | [`plugin-p0-validation.md`](plugin-p0-validation.md), [`plugin-p0-report.md`](plugin-p0-report.md), `experiments/workerd/plugins/*`                                                                                              | **Done 2026-11-10:** 45 checks, 27/27 gates passed; four conditions recorded in the report                                                          | 3d     |
| **P1** Artifacts and registry ✅        | Manifest schema, validator, digest store, install/enable/disable/rollback, DB migrations, admin API, **bundle delivery endpoint**                                                                | [`plugin-p1-plan.md`](plugin-p1-plan.md), [`plugin-p1-report.md`](plugin-p1-report.md); `schemas/plugin-manifest/`, `crates/plugin`, `migrations/store/0048`, `crates/api/routes/admin_plugins.rs`, `internal_plugin_runtime.rs` | Install/enable/disable/rollback works; invalid artifacts rejected; **no process restart**                                                           | 7d     |
| **P2** Runtime and capability bridge ✅ | Loader gateway, shared bridge, supervisor, tokens, KV, egress, `/apps/{id}` routing                                                                                                              | `crates/plugin/src/runtime/*`, `crates/plugin/assets/{gateway,bridge}.js`, `crates/api/src/routes/{plugin_api,plugin_runtime}.rs`, `apps/server` wiring; [`plugin-p2-report.md`](plugin-p2-report.md)                            | **Done 2026-11-10:** plugin loads in workerd, bridge calls work, direct fetch blocked, KV/egress/token checks pass, watchdog restarts a looped host | 6d     |
| **P3** Admin UI ✅                      | Target registry, `<PluginSlot>`, iframe bridge, nav and catch-all page, plugin management page, permissions; **six v1 targets**                                                                  | `apps/admin/src/plugins/*`, `apps/admin/src/pages/plugins.tsx`, [`plugin-p3-report.md`](plugin-p3-report.md)                                                                                                                     | **Done 2026-11-10:** six targets wired; extensions endpoint and signed UI entries; management page; 104 admin tests pass                            | 8d     |
| **P4** Storefront UI ✅                 | **Theme markers (comment + script forms) and fallback injector**, runtime.js, manifest generation, asset serving, default-template markers, chat example                                         | `crates/plugin/src/inject/*`, `crates/api/src/routes/storefront_extensions.rs`, [`plugin-p4-report.md`](plugin-p4-report.md)                                                                                                     | **Done 2026-11-10:** marker and fallback paths pass; `/_fastcart/extensions/*` serves assets; templates carry markers; chat example added           | 7d     |
| **P5** Events and jobs ✅               | Subscriptions, delivery, retries, dead letters, cron jobs, admin replay                                                                                                                          | `crates/plugin/src/dispatch/*`, `migrations/store/0049_plugin_dispatch.sql`, [`plugin-p5-report.md`](plugin-p5-report.md)                                                                                                        | **Done 2026-11-10:** outbox fanned out, delivery completed in workerd, cron jobs run, replay works; 2 s dispatcher tick                             | 5d     |
| **P6** Security and observability ✅    | Quotas, audit, metrics, logs, doctor                                                                                                                                                             | Metrics/log/health endpoints, `plugin doctor`; [`plugin-p6-report.md`](plugin-p6-report.md)                                                                                                                                      | **Done 2026-11-10:** structured plugin logs, RPC permission mapping, mount-token lifetime, `/health/plugins`, streaming and a raw WebSocket tunnel  | 4d     |
| **P7** DX and ecosystem ✅              | `init/dev/pack/doctor`, esbuild bundling, three templates, dev preview, SDK docs, example plugins                                                                                                | `packages/plugin-cli`, `packages/plugin-sdk`, `templates/plugin-*`; [`plugin-p7-report.md`](plugin-p7-report.md)                                                                                                                 | **Done 2026-11-10:** init/doctor/pack on all templates; TS + npm bundle; deterministic tar.gz; workerd dev preview with admin/storefront harness    | 6d     |
| **P8** Optional                         | Remote DOM-style UI, server-rendered slots, provider plugins, internal Wasm hooks, signing and marketplace                                                                                       | —                                                                                                                                                                                                                                | —                                                                                                                                                   | 8d+    |

**MVP = P0–P4 (≈31 person-days, 6–7 weeks).** It covers the core chat use case: admin configuration and conversation view, a storefront bubble, and server-side calls to an LLM and business APIs.

Dependencies: P0 → P1 → P2 → (P3 ∥ P4). P3 and P4 can run in parallel but both depend on P2 tokens and egress.

---

# 16. Code Changes (review only; nothing implemented)

All plugin Rust code lives under `crates/plugin/`. No new top-level crate is introduced.

```text
crates/plugin/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # public surface, re-exports
│   ├── manifest.rs             # plugin.json model, validation, targets, scopes
│   ├── capability.rs           # scope definitions and intersection rules
│   ├── registry.rs             # dependency resolution and enable ordering (moved from lib.rs)
│   ├── artifact/
│   │   ├── mod.rs
│   │   ├── validate.rs         # traversal, symlink, size, module graph
│   │   ├── store.rs            # content-addressed storage and GC
│   │   └── digest.rs
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── supervisor.rs       # workerd process lifecycle (reuse ssr-workerd patterns)
│   │   ├── generation.rs       # config generation and generation switching
│   │   ├── gateway.rs          # host → workerd HTTP client, streaming
│   │   └── loader.rs           # loader id, bundle delivery contract
│   ├── inject/
│   │   ├── mod.rs              # HTML injection entry point
│   │   ├── marker.rs           # marker parsing and precedence
│   │   └── html.rs             # streaming scanner and fallback
│   ├── dispatch/
│   │   ├── mod.rs
│   │   ├── events.rs           # outbox → delivery
│   │   └── jobs.rs             # scheduled and enqueued jobs
│   └── storage/
│       ├── mod.rs
│       ├── kv.rs
│       ├── settings.rs
│       ├── secrets.rs
│       └── tokens.rs
├── assets/
│   ├── gateway.js              # socket service, LOADER.get, routing
│   └── bridge.js               # forwards only /internal/plugins/v1/*
└── tests/                      # integration tests skip without the pinned binary
```

| Location                                                                                                   | Change                                                                                                       |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `crates/plugin/src/lib.rs`                                                                                 | Keep the registry API; add manifest v2 and the new modules                                                   |
| `crates/plugin/src/runtime/*` (new)                                                                        | Loader gateway, supervisor, generation switching                                                             |
| `crates/api/src/lib.rs`                                                                                    | Mount `/apps/{plugin}`, `/internal/plugins/v1`, `/_fastcart/plugin-ui`, `/_fastcart/extensions`              |
| `crates/api/src/routes/`                                                                                   | `admin_plugins.rs`, `internal_plugins.rs`, `internal_plugin_runtime.rs`, `plugin_assets.rs`, `store_apps.rs` |
| `crates/plugin/src/inject/*` (new)                                                                         | Marker scanning and streaming injection, called from the storefront route after cache lookup                 |
| `templates/storefront-*`, `apps/storefront`                                                                | Add head/body markers to root layouts (comment or script form)                                               |
| `schemas/storefront-artifact/manifest.schema.json`                                                         | Optional `extensions.markers` field for strict mode                                                          |
| `crates/platform` + `migrations/platform/0002_*.sql`                                                       | Three new permissions                                                                                        |
| `migrations/store/0048_plugins.sql`                                                                        | Section 10.2 tables                                                                                          |
| `apps/server/src/{bootstrap,state,shutdown}.rs`                                                            | Start/stop the plugin runtime, config, health aggregation                                                    |
| `apps/admin/src/plugins/*`, `pages/plugins.tsx`                                                            | Slot registry, iframe bridge, management page, nav merge, catch-all route                                    |
| `packages/plugin-sdk`, `packages/plugin-ui`, `packages/storefront-plugin-sdk`, `packages/plugin-cli` (new) | Client SDKs and CLI                                                                                          |
| `schemas/plugin-manifest/manifest.schema.json` (new)                                                       | JSON Schema                                                                                                  |
| `apps/cli/src/main.rs`                                                                                     | `plugin init/dev/pack/install/list/enable/disable/rollback/doctor`                                           |
| `.livingdocs/adr/0009-*.md`, `constitution.md`                                                             | New ADR; constitution amendment (plugin code executes in a supervised workerd process, never in the host)    |
| `README.md`, `.livingdocs/architecture/index.md`                                                           | Documentation index and deployment notes                                                                     |

Note: `crates/plugin` is currently a pure registry with no runtime. This work adds the runtime and redefines the manifest; it does not rewrite the registry from scratch.

---

# 17. Risks and Open Questions

## 17.1 Known risks

1. **No hard CPU limit in v1.** P0 confirmed the loader `limits` field is not enforced: a 300 ms sync loop ran to completion, and while one plugin spun for 3 s a healthy plugin took 2.9 s to answer. Mitigation: request deadlines plus a watchdog that restarts the plugin host, concurrency caps, and operator review. Per-plugin process isolation is the real fix and is deferred.
2. **`--experimental` dependency.** The Worker Loader is experimental and may change or disappear. Every workerd upgrade needs the P0 regression suite.
3. **`nodejs_compat` surface.** The allowlist must be verified module by module; `node:fs` and `node:net` must remain unavailable, and no disk/network binding may be added later without re-review.
4. **Storefront injection relies on `</body>`.** Themes that stream unusual HTML, omit closing tags, or compress at the worker level may skip injection. The injector must degrade silently, never break the response.
5. **Iframe visual consistency** is lower than Remote DOM. The host can only provide CSS variables and spacing conventions.
6. **Cross-store plugins.** A plugin installed on several stores is trusted across them; token scoping prevents accidental leaks but not a malicious plugin.
7. **Outbox is at-least-once.** Plugins must be idempotent; the SDK documents `event.id` prominently.

## 17.2 Open questions

| #   | Question                                                                   | Status                                                                       |
| --- | -------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| 1   | Which admin targets ship in v1?                                            | **Resolved (2026-11-10):** six targets; see 6.1                              |
| 2   | Is KV enough for conversation data, or is a `plugin_records` table needed? | **Resolved (2026-11-10):** KV only in v1; revisit after the chat pilot       |
| 3   | Should v1 allow plugins to write business data?                            | **Resolved (2026-11-10):** no; read-only scopes in v1                        |
| 4   | Are the quota defaults acceptable?                                         | **Resolved (2026-11-10):** approved as proposed; tunable after P2 load tests |
| 5   | Who owns the egress allowlist — author, merchant, or both?                 | Open. Current design: author declares, merchant confirms at install          |
| 6   | Should merchants choose `head:end` versus `body:end` per plugin?           | Open. v1 places embeds at `body:end`; the manifest field allows both         |

## 17.3 Review order

1. Confirm 6.1 targets and 6.2 storefront behavior.
2. Confirm 9.3 scope-to-endpoint mapping and the "no write by default" stance.
3. Confirm 10.2 schema (KV vs. records table).
4. Then P0 validation and script design.

---

# 18. References

**Shopify**

- App extensions: <https://shopify.dev/docs/apps/build/app-extensions>
- Apps in admin: <https://shopify.dev/docs/apps/build/admin>
- Admin UI extension targets: <https://shopify.dev/docs/api/admin-extensions/latest/targets>
- Build an admin block (Remote DOM, `shopify:admin/api/graphql.json`): <https://shopify.dev/docs/apps/build/admin/actions-blocks/build-admin-block>
- Theme app extensions: <https://shopify.dev/docs/apps/build/online-store/theme-app-extensions>
- App blocks and app embed blocks: <https://shopify.dev/docs/apps/build/online-store/theme-app-extensions/configuration>
- Shopify Functions (Wasm): <https://shopify.dev/docs/apps/build/functions>

**Medusa**

- Admin development: <https://docs.medusajs.com/learn/fundamentals/admin>
- Admin widgets and `defineWidgetConfig`: <https://docs.medusajs.com/learn/fundamentals/admin/widgets>
- Admin widget injection zones: <https://docs.medusajs.com/resources/admin-widget-injection-zones>
- Plugins: <https://docs.medusajs.com/learn/fundamentals/plugins>
- Creating and publishing a plugin: <https://docs.medusajs.com/learn/fundamentals/plugins/create>

**Runtime options**

- workerd: <https://github.com/cloudflare/workerd>
- Worker Loader API (upstream header): <https://github.com/cloudflare/workerd/blob/main/src/workerd/api/worker-loader.h>
- celld: <https://github.com/denoland/celld> and <https://celld.dev>
- celld security boundary: <https://github.com/denoland/celld/blob/main/docs/security.md>
- celld limitations: <https://github.com/denoland/celld/blob/main/docs/limitations.md>
- componentize-js (JS → Wasm component): <https://github.com/bytecodealliance/componentize-js>
- Wasmtime (fuel, epochs, component model): <https://docs.wasmtime.dev/>

**This repository**

- [ADR 0007 — Universal SSR runtime with workerd](adr/0007-universal-ssr-runtime-with-workerd.md)
- [Universal SSR Runtime plan](universal-ssr-runtime.md)
- [ADR 0008 — Themes as deployment artifacts](adr/0008-themes-as-deployment-artifacts.md)
- `schemas/storefront-artifact/manifest.schema.json` (existing artifact manifest pattern)
- `crates/ssr-workerd/src/{process,gateway}.rs`, `src/runtime/config_generator.rs`

---

# Appendix A: Runtime Substrate Reassessment (2026-11-10)

> Trigger: "Is workerd too heavy and inflexible? Can celld give more control? What are the trade-offs of a Wasm plugin system?"
>
> This appendix records the analysis behind the decision log. It does not change the extension-point design, which is substrate-independent.

## A.1 Separating the objections

| Objection                                                       | Valid?                       | Notes                                                                   |
| --------------------------------------------------------------- | ---------------------------- | ----------------------------------------------------------------------- |
| An extra process and ~85 MB                                     | Yes                          | Fixed cost, nearly independent of plugin count (10 plugins added 11 MB) |
| Installing a plugin requires regenerating config and restarting | Yes, for the original design | 0.2–0.5 s per restart, briefly affecting every plugin in the process    |
| No per-plugin CPU cap                                           | Yes                          | The capnp schema has no `limits` field for workers                      |
| JavaScript/TypeScript only                                      | Yes                          | workerd executes JS/Wasm modules, which constrains authoring            |
| Slow request path                                               | No                           | Measured p50 0.43 ms, p95 0.55 ms                                       |

The second item was the core of "inflexible", and its premise (static services) no longer holds: see A.3.

## A.2 celld

[celld](https://github.com/denoland/celld) (Apache-2.0, Rust, created 2025-04, ~6.2k stars, v0.6.x beta) is a **self-hosted Cloudflare Workers/Durable Objects platform**, not a plugin runtime:

- One process embeds V8 and executes Wrangler bundles.
- Workers, Durable Objects, KV, Queues, D1, R2, Workflows, Cron, and static assets.
- Each Durable Object is a **cell**: a named server with its own SQLite database.
- A fleet shares an S3/GCS/Azure bucket; conditional bucket writes grant cell ownership without a consensus service.
- A single-node durable write is ~90 ms; with a second node it is ~25 ms.

What matters here is its **Dynamic Workers** API: `env.LOADER.get(id, () => ({ modules, env, globalOutbound, limits, tails }))`, runtime-loaded code with per-worker limits and capabilities.

### What celld genuinely offers

1. **Lifecycle:** runtime loading and unloading without config changes or restarts. This is the flexibility gap in the original workerd design.
2. **Resources:** `cpuMs` and `subRequests` in `WorkerCode.limits` (celld states it enforces them), a per-isolate V8 heap cap (default 128 MB), resident-cell limits, and memory-pressure shedding.
3. **State:** a plugin implemented as a Durable Object gets SQLite, alarms, hibernation, and migration for free.

### Why it is not the substrate

| Cost                                                 | Impact                                                                                                                                                       |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Beta and explicitly unsafe for hostile multi-tenancy | Its security doc states it is not safe for hostile tenants; isolates are not an OS boundary                                                                  |
| One fleet runs one application                       | No account service or multi-tenant scheduler; plugins can only be dynamic workers inside the same application                                                |
| Production requires object storage                   | Only `celld dev` uses local storage; production needs S3/GCS/Azure, conflicting with FastCart's single-node, SQLite-first principle                          |
| Platform on top of a platform                        | It ships routing, deployment (`wrangler.json`, `celld deploy`), leases, scheduling, and no TLS; FastCart would either hand over ingress or add another layer |
| Operational surface                                  | Multi-node fleets, bucket credentials, plaintext peer HTTP (private network or WireGuard), alpha operator API with mostly unauthenticated routes             |
| Evolution                                            | PRs are closed; upgrades stop the whole fleet; mixed versions cannot coexist                                                                                 |

**Conclusion:** celld is a useful reference implementation that proves runtime loading, per-plugin limits, and per-plugin state are necessary. Adopting it would make FastCart **heavier**, not lighter, for a single-node self-hosted product. The model to borrow is the Worker Loader.

## A.3 Measured: the pinned workerd already ships the Worker Loader

Verified against `workerd v1.20261109.1` from this repository:

| Test                                            | Result                                                                                                                                                                                                                              |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Without `--experimental`                        | Config load fails: "Worker loader bindings are an experimental feature … must run workerd with `--experimental`"                                                                                                                    |
| With `--experimental`, load a plugin            | `env.LOADER.get(id, getCode)` and `getEntrypoint().fetch()` both work                                                                                                                                                               |
| Plugin receives a service binding through `env` | Works: it can call the host API                                                                                                                                                                                                     |
| `globalOutbound: null`                          | Works: `fetch("https://example.com")` throws a clear "not permitted to access the internet" error                                                                                                                                   |
| `limits: { cpuMs: 100, subRequests: 5 }`        | **Accepted but not enforced** (P0 V10–V13): a 300 ms synchronous loop completed, eight consecutive subrequests succeeded, and entrypoint-level limits did not interrupt either. A 3 s loop delayed another plugin by 2.9 s (P0 V19) |

The pinned build also contains an experimental `Config.cluster` mode (S3 bucket, `workerd deploy`), overlapping celld's positioning. It is unnecessary for a single-node deployment but shows workerd acquiring platform features.

Implications: drop the "install = regenerate config + restart" step, load plugins with `LOADER.get`, and treat `limits` as advisory only.

## A.4 Wasm: the trade-off

Shape: Wasmtime embedded in Rust, a WIT world, host imports for HTTP/KV/log/store queries, and one component instance per plugin.

| Pros                                                                                       | Cons                                                                                                       |
| ------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| No extra process; single-binary deployment preserved                                       | The host ABI and SDKs must be built (4–6 weeks)                                                            |
| Microsecond instantiation; pooling allocator available                                     | JavaScript authors need componentize-js: ~8 MB SpiderMonkey per component, experimental, sync-only imports |
| Reliable hard limits: fuel (deterministic CPU), epoch (wall-clock), `StoreLimits` (memory) | No DOM; the UI still needs the browser                                                                     |
| Capability security is structural: no syscalls, no filesystem, no network unless imported  | Async imports, WebSockets, SSE, streaming, and background tasks all need host plumbing                     |
| Language-agnostic (Rust, Go, C, Zig, AssemblyScript)                                       | No npm ecosystem; dependencies must compile to Wasm                                                        |
| Deterministic, auditable, small artifacts, easy to sign                                    | Weak debugging and observability; versioned WIT worlds are a long-term burden                              |
| Portable across hosts and test harnesses                                                   | Ill-suited to full applications such as a chat backend                                                     |

Best fit: synchronous deterministic hooks (discounts, pricing, tax, validation, webhook transforms), which is exactly Shopify Functions' scope. Not the right fit for third-party application plugins.

## A.5 Comparison

| Dimension              | workerd (original) | workerd + Loader (chosen)  | celld               | Wasmtime components                     |
| ---------------------- | ------------------ | -------------------------- | ------------------- | --------------------------------------- |
| Extra process          | 1 (~85 MB)         | 1 (~85 MB)                 | 1 platform + bucket | None                                    |
| Plugin hot load        | No                 | Yes                        | Yes                 | Yes (swap module)                       |
| Per-plugin CPU cap     | No                 | Field exists, not enforced | Claimed implemented | Fuel + epochs                           |
| Per-plugin memory cap  | Process level      | Process level              | Per-isolate heap    | `StoreLimits`                           |
| Egress control         | `globalOutbound`   | `globalOutbound: null`     | Same                | None by default                         |
| Authoring language     | JS/TS/Wasm         | JS/TS/Wasm                 | JS/TS/Wasm          | Rust/Go/C/Zig/TS                        |
| State storage          | Host KV/SQLite     | Host KV/SQLite             | Per-plugin SQLite   | Host KV/SQLite                          |
| UI capability          | None (browser)     | None                       | None                | None                                    |
| Isolation boundary     | V8 isolate         | V8 isolate                 | V8 isolate          | Wasm linear memory                      |
| Operational complexity | Medium             | Medium                     | High                | Low                                     |
| Implementation effort  | Medium             | Medium                     | High                | High                                    |
| Maturity               | High               | Experimental feature       | Beta                | Core stable; JS components experimental |

## A.6 Validation checklist before P0

1. Does the loader still exist and behave the same in the next pinned release? What is the upgrade process for `--experimental`?
2. Are `limits.cpuMs` and `subRequests` enforced in a newer build or under a different call path? If not, how do we terminate a synchronous infinite loop? (The only hard answer is process isolation, deferred.)
3. Concurrency model of a loaded worker: are calls to the same id serialized? Which thread runs the isolate? When does GC happen?
4. Compilation cost of the first `LOADER.get` for realistic module graphs, and the effective 64 MiB limit.
5. Exact `nodejs_compat` behavior for the allowlisted modules in this workerd build.
6. Wasmtime component-model async status and `wasi:http` availability, if the Wasm option is revisited.

---

## Appendix B: Relationship to the Previous `crates/plugin`

Before P1, the crate held only an in-memory `PluginRegistry` with a coarse `Capability` enum and dependency ordering. It had no runtime and no storage.

P1 replaced it with the implementation described in [plugin-p1-plan.md](plugin-p1-plan.md):

- The registry is the store database table `plugin_installations`, not an in-memory map.
- Capabilities are the fine-grained `Scope` list in `capability.rs`, with reserved write scopes rejected.
- Manifests are validated by `manifest.rs` against the enabled target list in `targets.rs`.
- Artifacts are stored by digest in `artifact/`, and installation state is managed by `service.rs`.

Dependency ordering between plugins was dropped because the v2 manifest does not declare dependencies. If cross-plugin dependencies are added later, they belong in a migration and the service layer, not in an in-memory registry.
