-- Theme lifecycle: versioned registrations, settings revisions and capability
-- bindings.
--
-- `themes.manifest_id` used to be globally unique. A theme manifest may now be
-- registered at multiple versions, so uniqueness moves to
-- `(manifest_id, version)`. SQLite cannot drop an inline UNIQUE constraint, so
-- the table is rebuilt while preserving every existing column and row.
--
-- New columns:
--   assets_manifest    validated assets manifest JSON loaded from the bundle
--   draft_settings     working settings that have not been published yet
--   settings_revision  number of the last published settings revision
--
-- The rebuild keeps foreign keys enforceable inside the migration transaction:
-- `theme_activations` is copied to a table that references `themes_new`, then
-- both old tables are dropped before the new tables are renamed into place.

CREATE TABLE themes_new (
    id TEXT PRIMARY KEY,
    manifest_id TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    runtime TEXT NOT NULL,
    manifest TEXT NOT NULL,
    bundle_path TEXT NOT NULL,
    settings_schema TEXT,
    settings TEXT NOT NULL DEFAULT '{}',
    is_active INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    editor_manifest TEXT,
    schema_version INTEGER NOT NULL DEFAULT 1,
    assets_manifest TEXT,
    draft_settings TEXT,
    settings_revision INTEGER NOT NULL DEFAULT 1,
    UNIQUE (manifest_id, version)
);

INSERT INTO themes_new (
    id, manifest_id, name, version, runtime, manifest, bundle_path,
    settings_schema, settings, is_active, created_at, updated_at,
    editor_manifest, schema_version, assets_manifest, draft_settings,
    settings_revision
)
SELECT
    id, manifest_id, name, version, runtime, manifest, bundle_path,
    settings_schema, settings, is_active, created_at, updated_at,
    editor_manifest, schema_version, NULL, NULL, 1
FROM themes;

CREATE TABLE theme_activations_new (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes_new (id) ON DELETE CASCADE,
    previous_theme_id TEXT,
    created_at INTEGER NOT NULL
);

INSERT INTO theme_activations_new (id, theme_id, previous_theme_id, created_at)
SELECT id, theme_id, previous_theme_id, created_at FROM theme_activations;

DROP TABLE theme_activations;
DROP TABLE themes;

ALTER TABLE themes_new RENAME TO themes;

-- Rebuild the activation table once more so its foreign key text always names
-- the final `themes` table. SQLite only rewrites references on RENAME when
-- foreign key enforcement is enabled, and migrations must behave identically
-- in both modes.
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

CREATE UNIQUE INDEX idx_themes_single_active
    ON themes (is_active) WHERE is_active = 1;

CREATE INDEX idx_theme_activations_created_at
    ON theme_activations (created_at);

-- Versioned settings snapshots. Draft rows are working copies; published rows
-- are the history that settings restores read from.
CREATE TABLE theme_settings_revisions (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes (id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    settings TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'published')),
    created_by TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (theme_id, revision)
);

CREATE INDEX idx_theme_settings_revisions_theme
    ON theme_settings_revisions (theme_id, revision DESC);

-- Seed the current settings of every pre-existing theme as its first
-- published revision so the history is complete after the upgrade.
INSERT INTO theme_settings_revisions (
    id, theme_id, revision, settings, status, created_by, created_at
)
SELECT id || ':1', id, 1, settings, 'published', NULL, updated_at
FROM themes;

-- Runtime capability bindings supplied by plugins. `config` is a JSON object
-- and `endpoint` is an absolute URL validated by the service.
CREATE TABLE theme_capability_bindings (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes (id) ON DELETE CASCADE,
    capability_id TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    config TEXT NOT NULL DEFAULT '{}',
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (theme_id, capability_id)
);

CREATE INDEX idx_theme_capability_bindings_theme
    ON theme_capability_bindings (theme_id);
