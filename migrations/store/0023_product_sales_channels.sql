-- Products are published to one or more sales channels.

CREATE TABLE IF NOT EXISTS product_sales_channels (
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    sales_channel_id TEXT NOT NULL REFERENCES sales_channels (id) ON DELETE CASCADE,
    PRIMARY KEY (product_id, sales_channel_id)
);

CREATE INDEX IF NOT EXISTS idx_product_sales_channels_channel
    ON product_sales_channels (sales_channel_id);
