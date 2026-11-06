---
type: Issue
title: Migrate the admin page editor to Puck
description: Adopt Puck for the admin canvas while keeping PageBody as the canonical schema.
status: open
labels: [area:admin]
timestamp: 2026-11-06T16:18:26Z
---

## 0002. Migrate the admin page editor to Puck

Implements [ADR 0004](/adr/0004-puck-as-the-admin-page-editor.md) and the admin requirement in [PRD 0001](/prd/0001-fastcart-v1-platform-requirements.md). The current editor is a bespoke block tree, palette and field inspector. This work adopts Puck for the canvas while keeping the versioned `PageBody` as the canonical page schema.

### Scope

- Add `@measured/puck` to the admin and host it on the existing page-editor route.
- Adapt `editor.manifest.json` to a Puck `Config`: categories, components, fields, default props, slots and zones.
- Map Puck data bidirectionally to `PageBody` (`content`, `root.props`, `zones`, nested `slots`) with stable block IDs and no database migration.
- Reuse the existing data-source, asset and inquiry-form pickers through Puck custom fields.
- Keep the JSON view, draft revisions, optimistic concurrency, signed preview, publish and version restore.
- Remove the bespoke palette, block tree and field inspector only after parity is verified.

### Decision

The adapter lives in the admin application, not in the API or the theme SDK. The server continues to validate `PageBody`; Puck is an editing surface, never the authoritative schema.

### Acceptance

- Existing pages with content blocks, zones and nested slots load in Puck and round-trip to the same `PageBody` shape.
- Adding, reordering, editing and deleting a block through Puck persists after reload.
- Data-source, asset and inquiry-form fields still resolve through the platform pickers.
- Preview, publish, version restore and stale-revision conflict behavior still work.
- Admin typecheck, lint, build and tests pass, including the page-editor tests.

### Plan

1. Add Puck and a read-only host that renders existing pages and saves unchanged data.
2. Implement the manifest-to-config adapter for categories, fields and default props.
3. Implement the `PageBody` ↔ Puck data adapter, including zones and nested slots.
4. Port custom fields to Puck, reusing the existing pickers.
5. Port save, dirty-state, preview, versions and conflict handling.
6. Run a parity review, remove the old editor components and update tests/docs.
