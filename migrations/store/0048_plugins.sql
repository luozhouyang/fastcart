-- Plugins: installations, settings, secrets, key-value storage and delivery state.

CREATE TABLE IF NOT EXISTS plugin_installations (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    version TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0,
    settings_json TEXT NOT NULL DEFAULT '{}',
    settings_revision INTEGER NOT NULL DEFAULT 0,
    granted_scopes TEXT NOT NULL DEFAULT '[]',
    previous_digest TEXT,
    installed_by TEXT,
    installed_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (plugin_id)
);

CREATE INDEX IF NOT EXISTS idx_plugin_installations_enabled
    ON plugin_installations (enabled);

CREATE TABLE IF NOT EXISTS plugin_secrets (
    plugin_id TEXT NOT NULL,
    name TEXT NOT NULL,
    ciphertext BLOB NOT NULL,
    nonce BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (plugin_id, name)
);

CREATE TABLE IF NOT EXISTS plugin_kv (
    plugin_id TEXT NOT NULL,
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    expires_at INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (plugin_id, namespace, key)
);

CREATE INDEX IF NOT EXISTS idx_plugin_kv_expiry ON plugin_kv (expires_at);

CREATE TABLE IF NOT EXISTS plugin_event_subscriptions (
    plugin_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (plugin_id, event_type)
);

CREATE TABLE IF NOT EXISTS plugin_deliveries (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at INTEGER NOT NULL,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (plugin_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_plugin_deliveries_claim
    ON plugin_deliveries (status, available_at);

CREATE TABLE IF NOT EXISTS plugin_jobs (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL,
    name TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at INTEGER NOT NULL,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plugin_jobs_claim
    ON plugin_jobs (status, available_at);

CREATE TABLE IF NOT EXISTS plugin_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id TEXT NOT NULL,
    level TEXT NOT NULL,
    message TEXT NOT NULL,
    fields_json TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plugin_logs_plugin
    ON plugin_logs (plugin_id, created_at);
