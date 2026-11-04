-- Theme editor contracts and optimistic page revisions.
--
-- `editor_manifest` stores the validated editor.manifest.json payload for a
-- registered theme. `revision` is the optimistic concurrency token for page
-- updates. `kind` separates draft snapshots from published snapshots, while
-- `origin` records whether a human or an AI task produced the snapshot.

ALTER TABLE themes ADD COLUMN editor_manifest TEXT;
ALTER TABLE themes ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1;

ALTER TABLE pages ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;

ALTER TABLE page_versions ADD COLUMN kind TEXT NOT NULL DEFAULT 'published';
ALTER TABLE page_versions ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual';

CREATE INDEX IF NOT EXISTS idx_page_versions_kind
    ON page_versions (page_id, kind, version);
