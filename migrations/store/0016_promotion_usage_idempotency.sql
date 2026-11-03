-- Prevent duplicate usage rows when checkout completion is retried.

CREATE UNIQUE INDEX IF NOT EXISTS idx_promotion_usage_order
    ON promotion_usage (promotion_id, order_id)
    WHERE order_id IS NOT NULL;
