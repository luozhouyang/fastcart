# Plugin System P4 Report: Storefront UI Injection

**Date:** 2026-11-10
**Status:** Implemented with streaming injection.
**Companion documents:** [Plugin System design](workerd-plugin-system.md), [P2 report](plugin-p2-report.md), [P3 report](plugin-p3-report.md).

---

## 1. Injection

`crates/plugin/src/inject/` scans HTML byte-wise:

- comment markers `<!--fastcart:extensions-head-->` and `<!--fastcart:extensions-body-->`;
- script markers `<script type="application/fastcart-extensions" data-slot="head|body"></script>`;
- fallback to the last `</head>` / `</body>`;
- sentinel `<!--fastcart:extensions-injected-->` makes a second pass byte-identical;
- fragments (no `<!doctype`/`<html` in the first 1 KB) and non-HTML responses are skipped;
- `Content-Encoding` other than identity skips injection;
- `StreamInjector` scans chunks incrementally with a 32 KB tail, so a marker split across chunks is still found and output starts before EOF.

Only full documents returned through the storefront fallback are transformed. The theme's original HTML remains unchanged; the host writes only tags derived from validated manifest fields.

## 2. Storefront manifest and assets

`crates/api/src/routes/storefront_extensions.rs`:

- builds the enabled-plugin list per request, filtered by `pages` and `exclude_paths`;
- emits CSS `<link>` tags into the head and an inline manifest plus `runtime.js` into the body;
- serves `/_fastcart/extensions/runtime.js`, `/_fastcart/extensions/manifest.json`, and digest-scoped boot assets with immutable caching;
- asset requests are store-resolved through `Host`, verified against `plugin_installations`, and read through the traversal-safe artifact reader;
- the runtime mounts each `entry` as a module and exposes `window.FastCartExtensions`.

`crates/api/assets/extensions-runtime.js` is the versioned boot runtime. It prefers the inline manifest and falls back to `manifest.json` when a strict CSP removes inline scripts.

## 3. Theme markers

Markers were added to the repository templates:

| Template               | Form                                      |
| ---------------------- | ----------------------------------------- |
| `storefront-tanstack`  | script markers in the root route          |
| `storefront-astro`     | comment markers in `index.astro`          |
| `storefront-sveltekit` | comment markers in `app.html`             |
| `storefront-generic`   | comment markers in the worker HTML string |

The chat example (`templates/plugin-chat/`) uses a Shadow DOM bubble and calls `/apps/chat-widget/api/hello`; its admin settings page speaks the P3 `postMessage` protocol.

## 4. Verification

| Check                                              | Result                                                                                                          |
| -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `cargo test -p fastcart-plugin --lib inject`       | 9 injection tests (markers, fallback, idempotency, fragment, one-byte streaming, sentinel)                      |
| `cargo test -p fastcart-api --test storefront_ssr` | injection E2E: full document + enabled embed produces the sentinel, inline manifest, runtime script and CSS URL |
| Existing storefront tests                          | pass                                                                                                            |

## 5. Limitations

| Limitation                                                     | Impact                                                                                          | Owner |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ----- |
| WebSocket message size is capped at 1 MiB                      | Larger frames close the tunnel                                                                  | P6    |
| Inline manifest can be blocked by a strict CSP without `nonce` | Runtime falls back to `manifest.json`; the inline form is skipped only if the page has no nonce | P6    |
| No `slot`, `block`, or server-rendered extension kinds         | v1 only supports `embed`                                                                        | P8    |
| Manifest query per HTML response                               | One DB query per storefront page; no per-store cache yet                                        | P6    |
| No performance-budget enforcement                              | Large plugin boot files are not blocked                                                         | P6    |
| Chat example is a template only                                | Not packaged into the default artifact corpus                                                   | P7    |

## 6. Follow-ups

- Reuse the injection scanner for the streaming path once the storefront body is streamed.
- Add a per-store extension manifest cache keyed by plugin generation.
- Add CSP nonce support and an external-manifest fallback test.
