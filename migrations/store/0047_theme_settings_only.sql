-- Themes are deployment artifacts (ADR 0008). The registry no longer stores or
-- validates theme bundles: a theme record is manifest metadata
-- (`manifest_id`, `name`, `version`), a settings schema, the settings and their
-- history, plus activation state. Rendering and assets belong to a deployed SSR
-- artifact.
--
-- Dropped columns:
--   runtime, manifest, bundle_path, editor_manifest, schema_version,
--   assets_manifest
--
-- The `themes` table is rebuilt because SQLite cannot drop columns that carry
-- inline constraints and this keeps `(manifest_id, version)` unique. Child
-- tables are rebuilt too: with foreign key enforcement on, `DROP TABLE themes`
-- performs an implicit `DELETE FROM` that would cascade into
-- `theme_activations` and `theme_settings_revisions` and erase that history.
-- Both tables keep exactly their previous columns and rows.
--
-- The rebuild follows `0044_theme_lifecycle.sql`: children first reference
-- `themes_new`, then the old tables are dropped, the new table is renamed into
-- place and the children are rebuilt once more so their foreign key text names
-- the final `themes` table in both foreign key modes.

CREATE TABLE themes_new (
    id TEXT PRIMARY KEY,
    manifest_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    settings_schema TEXT,
    settings TEXT NOT NULL DEFAULT '{}',
    is_active INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    draft_settings TEXT,
    settings_revision INTEGER NOT NULL DEFAULT 1,
    UNIQUE (manifest_id, version)
);

INSERT INTO themes_new (
    id, manifest_id, name, version, settings_schema, settings, is_active,
    created_at, updated_at, draft_settings, settings_revision
)
SELECT
    id, manifest_id, name, version, settings_schema, settings, is_active,
    created_at, updated_at, draft_settings, settings_revision
FROM themes;

CREATE TABLE theme_activations_new (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes_new (id) ON DELETE CASCADE,
    previous_theme_id TEXT,
    created_at INTEGER NOT NULL
);

INSERT INTO theme_activations_new (id, theme_id, previous_theme_id, created_at)
SELECT id, theme_id, previous_theme_id, created_at FROM theme_activations;

CREATE TABLE theme_settings_revisions_new (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes_new (id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    settings TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'published')),
    created_by TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (theme_id, revision)
);

INSERT INTO theme_settings_revisions_new (
    id, theme_id, revision, settings, status, created_by, created_at
)
SELECT id, theme_id, revision, settings, status, created_by, created_at
FROM theme_settings_revisions;

DROP TABLE theme_activations;
DROP TABLE theme_settings_revisions;
DROP TABLE themes;

ALTER TABLE themes_new RENAME TO themes;

-- Rebuild the activation and revision tables once more so their foreign keys
-- always name the final `themes` table. SQLite only rewrites references on
-- RENAME when foreign key enforcement is enabled, and migrations must behave
-- identically in both modes.
CREATE TABLE theme_activations_final (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes (id) ON DELETE CASCADE,
    previous_theme_id TEXT,
    created_at INTEGER NOT NULL
);

INSERT INTO theme_activations_final (id, theme_id, previous_theme_id, created_at)
SELECT id, theme_id, previous_theme_id, created_at FROM theme_activations_new;

DROP TABLE theme_activations_new;

ALTER TABLE theme_activations_final RENAME TO theme_activations;

CREATE TABLE theme_settings_revisions_final (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes (id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    settings TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'published')),
    created_by TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (theme_id, revision)
);

INSERT INTO theme_settings_revisions_final (
    id, theme_id, revision, settings, status, created_by, created_at
)
SELECT id, theme_id, revision, settings, status, created_by, created_at
FROM theme_settings_revisions_new;

DROP TABLE theme_settings_revisions_new;

ALTER TABLE theme_settings_revisions_final RENAME TO theme_settings_revisions;

CREATE UNIQUE INDEX idx_themes_single_active
    ON themes (is_active) WHERE is_active = 1;

CREATE INDEX idx_theme_activations_created_at
    ON theme_activations (created_at);

CREATE INDEX idx_theme_settings_revisions_theme
    ON theme_settings_revisions (theme_id, revision DESC);

-- Runtime capability bindings belonged to the legacy bundle runtime and are no
-- longer part of the registry. Dropping the table also drops its index.
DROP TABLE theme_capability_bindings;
