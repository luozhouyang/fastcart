---
type: ADR
title: Storefront themes as deployment artifacts
description: A theme is an SSR artifact plus settings, not a bundle loaded by a Node/V8 runtime adapter.
owner: luozhouyang
status: Accepted
implementation: partial
timestamp: 2026-11-09T22:05:00Z
---

# 0008. Storefront themes as deployment artifacts

## Context

The theme system predates the universal SSR runtime. Today a theme is a trusted
bundle registered with `bundle_path`, an entry module, a component editor
manifest and dynamic routes. Rust resolves the page, the page bindings, the
theme data and the capability grants, serialises a `RuntimeRequest`, and one of
two adapters renders it: the Node sidecar (working) or the embedded V8 adapter
(a stub). Assets are served from the bundle path.

ADR 0007 replaced that execution model with framework-built Worker artifacts,
but left the theme stack untouched. The result is two rendering contracts and
two deployment stories for the same product surface:

- a **framework storefront** deployed as a `fastcart-ssr.json` artifact and run
  by `workerd`, with the artifact fetching commerce data itself;
- a **theme bundle** registered in the admin, rendered by a Node/V8 adapter,
  with Rust pre-resolving all data.

Keeping both costs a second renderer, a sidecar, a legacy internal HTTP surface,
a runtime switch (`storefront.runtime = "v8" | "node"`), a theme-provided editor
runtime and two asset pipelines. It also means the theme's rendering contract
cannot evolve without dragging the legacy adapters along.

## Decision

1. **A theme is a storefront artifact.** A theme is a frontend project whose
   build produces a `fastcart-ssr.json` artifact, deployed through the universal
   deployment API and served by `workerd`. FastCart does not know the framework
   used to build it.
2. **The theme registry keeps settings, not rendering.** A theme record keeps
   its identity, version, settings schema, published settings, settings
   revisions and activation state. `bundle_path`, `entry`, `routes`,
   `assets_manifest`, `editor_manifest` and `runtime` are removed; assets come
   from the artifact and are served by the universal asset routes.
3. **The artifact owns rendering and data access.** Page content, products,
   collections and theme settings are fetched by the artifact through the
   storefront SDK (`/api/v1/store/*`, or `/internal/ssr/v1/stores/{id}/*` with
   the scoped internal token). Rust no longer resolves page bindings, theme
   routes or capability payloads for theme traffic.
4. **Preview keeps working through preview deployments.** A theme preview is a
   preview deployment plus the existing signed preview-asset authorization; no
   separate theme preview protocol is introduced.
5. **The legacy runtime line is removed.** `fastcart-runtime-api`,
   `fastcart-runtime-node`, `fastcart-runtime-pool`, `fastcart-runtime-v8`,
   `fastcart-runtime-bridge`, `apps/theme-sidecar`, the `storefront.runtime`
   switch, the secret-gated `/internal/storefront` surface, the theme asset
   route and the theme-bundle/editor runtime contracts are deleted.
6. **Page content survives as CMS data.** The page and block content model may
   stay as data that an artifact can fetch and render. The admin page editor is
   reworked independently; it no longer loads a theme-provided editor bundle or
   binds blocks to theme components.

The consequence is accepted deliberately: merchants lose the theme-provided
component editor. Theme customization becomes "build a storefront with the SDK
and edit settings in the admin", which is the same model the framework templates
already use.

## Consequences

**Gained**

- One rendering contract, one deployment path, one asset pipeline, one preview
  mechanism.
- The admin stores theme settings; it does not execute theme code.
- Deleting five crates, a sidecar process and a legacy HTTP surface removes a
  large amount of security-sensitive code (secret-gated internal route, worker
  limits, bundle validation, runtime switching).

**Accepted trade-offs**

- Existing theme bundles must be rebuilt as artifacts before they can serve
  traffic. Until then the storefront requires an artifact, exactly as the
  universal runtime already requires.
- Component-level visual editing is gone. A generic page editor may return
  later (see [ADR 0004](0004-puck-as-the-admin-page-editor.md)), but it will
  edit content, not theme components.
- `packages/theme-sdk` loses the runtime mirror types; the editor manifest
  schema is retired.

## Migration

1. **Decision and dead code.** Record this ADR; delete `runtime-v8` and
   `runtime-bridge`, which have no callers.
2. **Remove legacy execution.** Delete `runtime-api`, `runtime-node`,
   `runtime-pool`, `apps/theme-sidecar`, the `storefront.runtime` switch, the
   legacy render fallbacks in `storefront.rs` / `preview.rs`, the
   `/internal/storefront` surface, the theme asset route and the legacy runtime
   knobs in configuration.
3. **Reshape the theme registry.** Drop bundle/editor/runtime fields with a store
   migration; registration accepts manifest metadata, settings schema and
   initial settings; the store theme endpoint returns settings only.
4. **Rework the admin.** The theme screen edits settings and activation; the
   block/component editor is retired with the theme editor contract.
5. **Retire `packages/theme-sdk` bundle types** and document the artifact-based
   theme workflow in the storefront templates.

## Implementation status

- [x] ADR recorded; `runtime-v8` and `runtime-bridge` deleted.
- [x] Legacy execution line removed: `runtime-api`, `runtime-node`,
      `runtime-pool`, `apps/theme-sidecar`, `storefront.runtime`, the legacy
      render/preview fallbacks and `fastcart-storefront-data`.
- [x] Theme registry stores settings only (store migration
      `0047_theme_settings_only.sql`); bundle validation, capability bindings
      and the theme asset route are gone.
- [x] Admin theme screen converged to settings, activation and preview; the
      block editor uses a built-in generic palette instead of a theme-provided
      editor manifest.
- [x] `packages/theme-sdk` and the `themes/default` bundle retired in favour of
      the artifact templates.
- [x] The secret-gated `/internal/storefront` surface and
      `FASTCART_INTERNAL_FETCH_SECRET` are removed; artifacts use the scoped
      `/internal/ssr/v1` credential surface.

## Verification

- No crate depends on `fastcart-runtime-*`; `crates/storefront-runtime` and
  `apps/theme-sidecar` no longer exist.
- `storefront.runtime`, `FASTCART_STOREFRONT_ENABLED` and the internal
  storefront secret are absent from configuration, README and `.env.example`.
- A theme artifact serves a storefront through `workerd`; theme settings are
  readable through the store API; preview deployments serve preview assets.
- `cargo test --workspace`, clippy with `-D warnings` and the frontend
  typecheck/build pass.
