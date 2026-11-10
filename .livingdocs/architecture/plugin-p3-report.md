# Plugin System P3 Report: Admin UI Extension Points

**Date:** 2026-11-10
**Status:** Implemented. Two UI-level limitations are recorded below.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P2 report](plugin-p2-report.md), [P4 report](plugin-p4-report.md).

---

## 1. Backend

| Surface                                                     | Behavior                                                                                                                                                                                                    |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /api/v1/admin/plugins/extensions`                      | Returns a flat list of admin extensions across enabled plugins, each with target, label/title/icon/route, signed `src`, height, priority, required permission, plugin id/name/version/digest, and settings. |
| `GET /_fastcart/plugin-ui/{store}/{plugin}/{digest}/{path}` | Serves artifact UI files. HTML entries require `?t=` mount tokens (5 min, installation-derived); assets are immutable and digest-scoped. HTML responses carry a sandbox CSP with `connect-src 'none'`.      |
| `POST /api/v1/admin/plugins/{id}/rpc`                       | Declared-method allowlist from `manifest.rpc`, admin session + CSRF + `plugins.read`, 10 s deadline, 512 KB cap.                                                                                            |
| `GET /api/v1/admin/plugins/runtime`                         | Runtime health for operators.                                                                                                                                                                               |

The iframe never receives the admin cookie: the frame uses `sandbox="allow-scripts allow-forms"` without `allow-same-origin`, and all data flows through the host RPC proxy.

---

## 2. Admin SPA

New code under `apps/admin/src/plugins/`:

- `runtime.tsx` — fetches the extension manifest once per store, refreshes on demand, hides everything in demo mode.
- `plugin-frame.tsx` — iframe lifecycle, `postMessage` channel, `init` payload (target, plugin, store, theme, context, user permissions, settings), RPC proxy, resize clamp, navigation, toasts, external links.
- `plugin-slot.tsx` — filters by target and permission, sorts by priority, renders at most three frames, renders nothing when empty.
- `types.ts` — extension shape.

New pages:

- `pages/plugins.tsx` — install archive (multipart), enable/disable, remove, inspect scopes, edit settings JSON.
- `pages/plugin-page.tsx` — full-page plugin route `/apps/{pluginId}` and `/apps/{pluginId}/...`.

Integration:

- The sidebar gets a Plugins entry and a dynamic "Plugins" group from `admin.nav.section` entries.
- Six v1 targets are wired: overview header, order details sidebar, product details sidebar, customer details sheet, settings page, and navigation.
- `PERMISSIONS.plugins.{read,write,install}` added; every control is permission-gated.

Routes are registered in `router.tsx`; `PluginExtensionsProvider` wraps the authenticated shell.

---

## 3. Verification

| Check                                             | Result                                                      |
| ------------------------------------------------- | ----------------------------------------------------------- |
| `cargo test -p fastcart-api --test admin_plugins` | 19 tests (includes extensions + signed UI + undeclared RPC) |
| `pnpm --filter @fastcart/admin typecheck`         | clean                                                       |
| `pnpm --filter @fastcart/admin test`              | 104 passed                                                  |
| `pnpm --filter @fastcart/admin build`             | built in 2.4 s                                              |
| `pnpm exec eslint` on new admin files             | clean                                                       |

The extension test installs and enables a plugin, fetches `/plugins/extensions`, verifies the nav and settings-tab entries, loads the signed HTML entry (200), and confirms the same URL without a token returns 401.

---

## 4. Limitations

| Limitation                                              | Impact                                                                                                              | Owner             |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ----------------- |
| No component tests for `<PluginSlot>` / `<PluginFrame>` | The protocol is exercised only by typecheck and manual reasoning                                                    | P6 test hardening |
| RPC permission mapping is coarse                        | The token carries actor permissions, but the host does not map endpoint names such as `orders.get` to `orders.read` | P6                |
| Mount tokens last 5 minutes                             | Long-lived open iframes need a refresh/reload after expiry                                                          | P6                |
| Plugin page uses the nav entry or first HTML entry      | A plugin that wants several full pages needs more manifest surface                                                  | P8                |
| No resize throttling in the frame                       | A plugin can send resize events rapidly; the host clamps the height but does not throttle                           | P6                |

---

## 5. Next Inputs for P4

- Injection uses the same enabled-installation query and artifact store.
- The storefront embed entries are served from `/_fastcart/extensions/{plugin}/{digest}/...`, parallel to the admin UI route.
