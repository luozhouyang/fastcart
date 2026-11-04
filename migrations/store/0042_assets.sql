-- Store-scoped media asset metadata for the admin media library.
--
-- Binary payloads live in infrastructure storage; `storage_key` identifies the
-- object. Rows are soft-deleted so audit records and content bindings keep
-- resolving historical identifiers.

CREATE TABLE IF NOT EXISTS assets (
    id TEXT PRIMARY KEY,
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    storage_key TEXT NOT NULL UNIQUE,
    alt TEXT,
    width INTEGER CHECK (width IS NULL OR width >= 0),
    height INTEGER CHECK (height IS NULL OR height >= 0),
    created_by TEXT,
    created_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_assets_active_created
    ON assets (deleted_at, created_at);
