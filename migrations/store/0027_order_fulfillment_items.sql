-- Item-level fulfillments and shipping method snapshots on orders.

ALTER TABLE order_fulfillments ADD COLUMN provider_id TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE order_fulfillments ADD COLUMN provider_fulfillment_id TEXT;
ALTER TABLE order_fulfillments ADD COLUMN provider_data TEXT NOT NULL DEFAULT '{}';

CREATE TABLE IF NOT EXISTS order_fulfillment_items (
    id TEXT PRIMARY KEY,
    fulfillment_id TEXT NOT NULL REFERENCES order_fulfillments (id) ON DELETE CASCADE,
    order_item_id TEXT NOT NULL REFERENCES order_items (id) ON DELETE CASCADE,
    variant_id TEXT,
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    created_at INTEGER NOT NULL,
    UNIQUE (fulfillment_id, order_item_id)
);

CREATE INDEX IF NOT EXISTS idx_order_fulfillment_items_fulfillment
    ON order_fulfillment_items (fulfillment_id);

CREATE INDEX IF NOT EXISTS idx_order_fulfillment_items_order_item
    ON order_fulfillment_items (order_item_id);

ALTER TABLE orders ADD COLUMN shipping_methods TEXT NOT NULL DEFAULT '[]';
