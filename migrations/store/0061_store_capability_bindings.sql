-- Store-scoped bindings from a theme capability id to one enabled plugin
-- provider. Each store database is already scoped to one store, so the store
-- id is implicit.
CREATE TABLE IF NOT EXISTS store_capability_bindings (
    capability_id TEXT PRIMARY KEY,
    version INTEGER NOT NULL,
    plugin_id TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_store_capability_bindings_plugin
    ON store_capability_bindings (plugin_id);
