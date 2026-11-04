-- Durable obligations to purge asset blobs from object storage.
--
-- Deleting asset metadata commits a row here in the same transaction. The blob
-- is removed immediately when possible; otherwise the pending row is retried by
-- maintenance so a crash between the metadata commit and the storage delete
-- cannot leak the object forever.

CREATE TABLE IF NOT EXISTS asset_blob_purges (
    id TEXT PRIMARY KEY,
    storage_key TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    processed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_asset_blob_purges_pending
    ON asset_blob_purges (processed_at, created_at);
