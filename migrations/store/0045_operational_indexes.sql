-- Operational indexes for hot query paths. All are additive and idempotent so
-- existing stores can apply this migration without rewriting any data.

-- Restock and after-sales lookups select committed reservations by order.
-- The existing indexes lead with status, inventory item or checkout, so
-- filtering by order alone scans the reservation table.
CREATE INDEX IF NOT EXISTS idx_inventory_reservations_order
    ON inventory_reservations (order_id);

-- Catalog reference checks count order lines by variant. Without a leading
-- variant index every check scans the full order item history.
CREATE INDEX IF NOT EXISTS idx_order_items_variant
    ON order_items (variant_id);

-- Catalog reference checks count live cart lines by variant. The unique
-- (cart_id, variant_id) index cannot serve a variant-only lookup.
CREATE INDEX IF NOT EXISTS idx_cart_items_variant
    ON cart_items (variant_id);
