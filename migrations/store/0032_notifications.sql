-- Idempotent transactional notification deliveries.
--
-- One row is claimed per (event, template) pair before its email job is
-- enqueued. A row is only marked `sent` after the job is durably queued, so a
-- worker crash between claiming and enqueueing lets a later outbox retry try
-- again. `claim_token` identifies the attempt that owns an in-progress row so
-- a second attempt can neither release nor complete another attempt's claim.
-- Rows are retained for duplicate suppression and audit.

CREATE TABLE IF NOT EXISTS notification_deliveries (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    template TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'claimed', 'sent')),
    job_id TEXT,
    claim_token TEXT,
    claimed_at INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    created_at INTEGER NOT NULL,
    sent_at INTEGER,
    UNIQUE (event_id, template)
);

-- Supports reaping stale pending claims and operational reporting.
CREATE INDEX IF NOT EXISTS idx_notification_deliveries_status
    ON notification_deliveries (status, created_at);

CREATE INDEX IF NOT EXISTS idx_notification_deliveries_event_type
    ON notification_deliveries (event_type, created_at);
