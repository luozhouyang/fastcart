-- Configurable return reasons.
--
-- Administrators maintain a per-store list of return reasons. A return, claim or
-- exchange created with a `reason_id` snapshots the reason's label into its own
-- free-text `reason` column; the records intentionally have no foreign key to
-- this table, so deleting a reason never rewrites or breaks historical records.
-- The stored text is the record of what was chosen at creation time.

CREATE TABLE return_reasons (
    id TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    label TEXT NOT NULL,
    description TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_return_reasons_value ON return_reasons(value);
