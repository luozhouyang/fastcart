-- Carts and line items. Prices stored here are provisional snapshots that are
-- recalculated by the pricing service on every write and at checkout.

CREATE TABLE IF NOT EXISTS carts (
    id TEXT PRIMARY KEY,
    currency_code TEXT NOT NULL,
    customer_id TEXT,
    email TEXT,
    sales_channel_id TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    shipping_address TEXT NOT NULL DEFAULT '{}',
    billing_address TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    expires_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_carts_customer ON carts (customer_id);
CREATE INDEX IF NOT EXISTS idx_carts_status ON carts (status, expires_at);

CREATE TABLE IF NOT EXISTS cart_items (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts (id) ON DELETE CASCADE,
    product_id TEXT NOT NULL,
    variant_id TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    title TEXT NOT NULL,
    variant_title TEXT,
    sku TEXT,
    thumbnail TEXT,
    unit_price INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    is_tax_inclusive INTEGER NOT NULL DEFAULT 0,
    requires_shipping INTEGER NOT NULL DEFAULT 1,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (cart_id, variant_id)
);

CREATE INDEX IF NOT EXISTS idx_cart_items_cart ON cart_items (cart_id);

CREATE TABLE IF NOT EXISTS cart_addresses (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts (id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    first_name TEXT,
    last_name TEXT,
    company TEXT,
    address_1 TEXT,
    address_2 TEXT,
    city TEXT,
    province TEXT,
    postal_code TEXT,
    country_code TEXT,
    phone TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (cart_id, type)
);

CREATE TABLE IF NOT EXISTS cart_discounts (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts (id) ON DELETE CASCADE,
    promotion_id TEXT,
    code TEXT,
    amount INTEGER NOT NULL DEFAULT 0,
    is_automatic INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cart_discounts_cart ON cart_discounts (cart_id);

CREATE TABLE IF NOT EXISTS cart_shipping_methods (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts (id) ON DELETE CASCADE,
    shipping_option_id TEXT NOT NULL,
    name TEXT NOT NULL,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    is_tax_inclusive INTEGER NOT NULL DEFAULT 0,
    data TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cart_shipping_methods_cart
    ON cart_shipping_methods (cart_id);

CREATE TABLE IF NOT EXISTS cart_item_adjustments (
    id TEXT PRIMARY KEY,
    cart_item_id TEXT NOT NULL REFERENCES cart_items (id) ON DELETE CASCADE,
    promotion_id TEXT,
    code TEXT,
    amount INTEGER NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cart_shipping_method_adjustments (
    id TEXT PRIMARY KEY,
    shipping_method_id TEXT NOT NULL REFERENCES cart_shipping_methods (id) ON DELETE CASCADE,
    promotion_id TEXT,
    code TEXT,
    amount INTEGER NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS cart_tax_lines (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts (id) ON DELETE CASCADE,
    cart_item_id TEXT,
    shipping_method_id TEXT,
    name TEXT NOT NULL,
    rate_bps INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    is_tax_inclusive INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cart_tax_lines_cart ON cart_tax_lines (cart_id);
