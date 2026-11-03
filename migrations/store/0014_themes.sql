-- Themes: registry, settings and activation history.

CREATE TABLE IF NOT EXISTS themes (
    id TEXT PRIMARY KEY,
    manifest_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    runtime TEXT NOT NULL,
    manifest TEXT NOT NULL,
    bundle_path TEXT NOT NULL,
    settings_schema TEXT,
    settings TEXT NOT NULL DEFAULT '{}',
    is_active INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- At most one theme may be active per store.
CREATE UNIQUE INDEX IF NOT EXISTS idx_themes_single_active
    ON themes (is_active) WHERE is_active = 1;

CREATE TABLE IF NOT EXISTS theme_activations (
    id TEXT PRIMARY KEY,
    theme_id TEXT NOT NULL REFERENCES themes (id) ON DELETE CASCADE,
    previous_theme_id TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_theme_activations_created_at
    ON theme_activations (created_at);
