-- Orders lost the commercial context they were placed in, and the tax that was
-- collected only survived as a single aggregate.
--
-- `region_id` and `sales_channel_id` are copied from the cart so an order can
-- still be attributed to a region and a channel after the fact, which is what
-- tax reporting and channel reporting need.
--
-- `order_tax_lines` keeps the per-rate breakdown the cart already computed.
-- Collapsing it into `orders.tax_total` made the collected tax impossible to
-- reconcile against a rate — invoices and filings could not be produced.
-- `order_item_id` is null for a tax line levied on shipping. All statements are
-- additive so existing stores upgrade without rewriting data.

ALTER TABLE orders ADD COLUMN region_id TEXT;
ALTER TABLE orders ADD COLUMN sales_channel_id TEXT;

CREATE TABLE IF NOT EXISTS order_tax_lines (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    order_item_id TEXT REFERENCES order_items (id) ON DELETE CASCADE,
    shipping_option_id TEXT,
    name TEXT NOT NULL,
    rate_bps INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    is_tax_inclusive INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_tax_lines_order ON order_tax_lines (order_id);
