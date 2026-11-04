-- Durable store-level audit outbox.
--
-- Admin mutations write their audit entry here so a platform audit-log outage
-- does not silently drop the record. Maintenance drains pending rows to the
-- platform audit log and increments `attempts` on failure.
--
-- `data` and `last_error` may contain sensitive values: never log them.
-- Failed rows stay pending (no retry timestamp column) so the next maintenance
-- sweep retries them with a higher attempt count.

CREATE TABLE IF NOT EXISTS audit_outbox (
    id TEXT PRIMARY KEY,
    actor_id TEXT,
    store_id TEXT,
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    data TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    processed_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_audit_outbox_pending
    ON audit_outbox (processed_at, created_at);
