-- Store-wide return listing.
--
-- The existing `idx_order_returns_order` index is scoped to one order, so the
-- admin list that pages every return newest-first needs its own indexes. The
-- optional status filter is served by the composite index.

CREATE INDEX IF NOT EXISTS idx_order_returns_created
    ON order_returns (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_order_returns_status_created
    ON order_returns (status, created_at DESC, id DESC);
