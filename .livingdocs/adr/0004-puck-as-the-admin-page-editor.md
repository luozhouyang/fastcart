---
type: ADR
title: Puck as the admin page editor
description: Migrate the admin page editor to Puck while keeping PageBody as the canonical schema.
owner: luozhouyang
status: Accepted
timestamp: 2026-11-06T16:17:43Z
---

# 0004. Puck as the admin page editor

## Context

The admin page editor is a bespoke block tree with a component palette, field inspector, zone targets, nested slots, a data-source picker, JSON mode, draft revisions, iframe preview and publish. The theme/editor design notes target Puck as the editor while keeping the page body as the authoritative data model. The current editor works but owns layout, drag-and-drop behaviour and field rendering, which is a large surface to maintain and a barrier to the drag-and-drop UX merchants expect.

A decision is required because replacing the editor changes the admin dependency, the editor's interaction model and the test suite, but should not change the persisted page contract or SSR.

## Decision

We will migrate the admin page editor to `@measured/puck` and keep the existing versioned `PageBody` JSON as the canonical persisted model. A bidirectional adapter will map the active theme's `editor.manifest.json` into a Puck `Config` (categories, components, fields, default props, slots and zones) and map Puck data back into `PageBody` (`content`, `root.props`, `zones`, nested `slots`) without a database migration. Platform field types (`data_source`, `asset`, `custom`) keep their existing pickers through Puck custom fields. Server-side validation, draft revisions, optimistic concurrency, signed preview and publish remain authoritative and unchanged. The bespoke palette/tree/field editor is removed only after the Puck editor reaches feature parity for existing pages.

**Rejected alternatives:**

- **Keep the bespoke editor.** Rejected by the project owner; it continues to own drag-and-drop, accessibility and layout behaviour that Puck already provides.
- **Adopt Puck's data shape as the canonical page body.** Rejected because it would force a database migration, a published-snapshot migration and SSR/runtime changes for no product benefit.
- **Build a custom drag-and-drop layer on the existing editor.** Rejected because it duplicates Puck's core value while keeping the maintenance burden.

## Consequences

**Easier / gained:**

- Merchants get Puck's drag-and-drop canvas, component palette and field rendering.
- The platform keeps one canonical page schema for validation, previews, SSR and AI/API writers.
- The bespoke editor maintenance surface shrinks after the migration.

**Harder / accepted trade-offs:**

- Puck is a new front-end dependency with its own upgrade cadence and data-shape constraints.
- Zones and nested slots need an explicit adapter, including stable block IDs and legacy page compatibility.
- Existing editor tests must be rewritten around the Puck host or the adapter.

**Follow-ups:**

- Implement the adapter and Puck host behind the existing page route.
- Keep the JSON view, versions and preview dialog while the canvas changes.
- Remove the old editor components only after a parity review.
- Implementation is tracked in [Issue 0002](/issues/0002-migrate-the-admin-page-editor-to-puck.md).

## Verification

**Implementation impact:** `apps/admin/src/pages/page-editor.tsx`, `apps/admin/src/components/theme-editor/*`, `packages/admin-sdk`, `packages/shared-types`, `apps/admin/package.json`.

**Verification criteria:**

- An existing page with content blocks, zones and nested slots loads in Puck and saves back to the same `PageBody` shape.
- Adding, reordering, editing and deleting a block through Puck persists after reload.
- Data-source and asset fields still resolve through the platform pickers.
- Preview, publish, version restore and optimistic concurrency still work from the editor.
- Admin typecheck, lint, build and tests pass.
