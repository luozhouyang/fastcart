-- Lease-based outbox claiming prevents overlapping maintenance workers from
-- dispatching the same event twice.

ALTER TABLE outbox ADD COLUMN claimed_at INTEGER;
ALTER TABLE outbox ADD COLUMN claimed_by TEXT;
