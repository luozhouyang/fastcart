-- Indexes for cart aggregate reloads.
--
-- `CartService::load_aggregate` reads both adjustment tables by joining the
-- parent row and filtering on the foreign key column. SQLite does not index
-- foreign key columns automatically, so each of those reads degraded to a full
-- table scan — on every cart write, and therefore on every recalculation.
-- Both indexes are additive and idempotent so existing stores apply this
-- migration without rewriting data.

CREATE INDEX IF NOT EXISTS idx_cart_item_adjustments_item
    ON cart_item_adjustments (cart_item_id);

CREATE INDEX IF NOT EXISTS idx_cart_shipping_method_adjustments_method
    ON cart_shipping_method_adjustments (shipping_method_id);
