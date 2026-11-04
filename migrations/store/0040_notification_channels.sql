-- Store-scoped notification channels and idempotent channel deliveries.
--
-- `kind` is intentionally not constrained so new adapters (slack, dingtalk,
-- wecom) can be added without a table rebuild. Status values are stable.
-- Secrets live in dedicated columns, never in the generic settings table.

CREATE TABLE IF NOT EXISTS notification_channels (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    endpoint_url TEXT NOT NULL,
    signing_secret TEXT,
    events TEXT NOT NULL DEFAULT '[]',
    config TEXT NOT NULL DEFAULT '{}',
    is_active INTEGER NOT NULL DEFAULT 1,
    deleted_at INTEGER,
    last_error TEXT,
    last_success_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notification_channels_active
    ON notification_channels (kind, is_active, deleted_at);

CREATE TABLE IF NOT EXISTS notification_channel_deliveries (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES notification_channels (id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'queued', 'sent', 'failed', 'skipped')),
    job_id TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    response_status INTEGER,
    response_body TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    sent_at INTEGER,
    UNIQUE (channel_id, event_id)
);

CREATE INDEX IF NOT EXISTS idx_channel_deliveries_status
    ON notification_channel_deliveries (status, created_at);
CREATE INDEX IF NOT EXISTS idx_channel_deliveries_channel
    ON notification_channel_deliveries (channel_id, created_at DESC);
