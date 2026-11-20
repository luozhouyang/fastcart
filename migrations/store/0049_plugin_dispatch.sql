-- Plugin delivery leases, job leases and retry metadata.

ALTER TABLE plugin_deliveries ADD COLUMN locked_at INTEGER;
ALTER TABLE plugin_deliveries ADD COLUMN locked_by TEXT;

CREATE INDEX IF NOT EXISTS idx_plugin_deliveries_claim
    ON plugin_deliveries (status, available_at);

ALTER TABLE plugin_jobs ADD COLUMN locked_at INTEGER;
ALTER TABLE plugin_jobs ADD COLUMN locked_by TEXT;
ALTER TABLE plugin_jobs ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 5;

CREATE INDEX IF NOT EXISTS idx_plugin_jobs_claim
    ON plugin_jobs (status, available_at);
