-- Checkout sessions and orders. Order rows preserve commercial snapshots.

CREATE TABLE IF NOT EXISTS checkout_sessions (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL UNIQUE,
    customer_id TEXT,
    email TEXT,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'created',
    shipping_address TEXT NOT NULL DEFAULT '{}',
    billing_address TEXT NOT NULL DEFAULT '{}',
    shipping_method TEXT NOT NULL DEFAULT '{}',
    totals TEXT NOT NULL DEFAULT '{}',
    promotion_codes TEXT NOT NULL DEFAULT '[]',
    payment_session_id TEXT,
    order_id TEXT,
    idempotency_key TEXT UNIQUE,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_checkout_status ON checkout_sessions (status, expires_at);

CREATE TABLE IF NOT EXISTS checkout_state_history (
    id TEXT PRIMARY KEY,
    checkout_id TEXT NOT NULL REFERENCES checkout_sessions (id) ON DELETE CASCADE,
    from_state TEXT,
    to_state TEXT NOT NULL,
    reason TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_checkout_history_checkout
    ON checkout_state_history (checkout_id, created_at);

CREATE TABLE IF NOT EXISTS orders (
    id TEXT PRIMARY KEY,
    display_number INTEGER NOT NULL,
    customer_id TEXT,
    email TEXT NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    payment_status TEXT NOT NULL DEFAULT 'awaiting',
    fulfillment_status TEXT NOT NULL DEFAULT 'not_fulfilled',
    refund_status TEXT NOT NULL DEFAULT 'not_refunded',
    subtotal INTEGER NOT NULL DEFAULT 0,
    discount_total INTEGER NOT NULL DEFAULT 0,
    shipping_total INTEGER NOT NULL DEFAULT 0,
    tax_total INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    shipping_address TEXT NOT NULL DEFAULT '{}',
    billing_address TEXT NOT NULL DEFAULT '{}',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    canceled_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_orders_display_number ON orders (display_number);
CREATE INDEX IF NOT EXISTS idx_orders_customer ON orders (customer_id);
CREATE INDEX IF NOT EXISTS idx_orders_status ON orders (status, created_at);
CREATE INDEX IF NOT EXISTS idx_orders_email ON orders (email);

CREATE TABLE IF NOT EXISTS order_items (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    product_id TEXT,
    variant_id TEXT,
    product_title TEXT NOT NULL,
    variant_title TEXT,
    sku TEXT,
    thumbnail TEXT,
    unit_price INTEGER NOT NULL,
    quantity INTEGER NOT NULL,
    subtotal INTEGER NOT NULL,
    discount_total INTEGER NOT NULL DEFAULT 0,
    tax_total INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL,
    is_tax_inclusive INTEGER NOT NULL DEFAULT 0,
    requires_shipping INTEGER NOT NULL DEFAULT 1,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_order_items_order ON order_items (order_id);

CREATE TABLE IF NOT EXISTS order_item_adjustments (
    id TEXT PRIMARY KEY,
    order_item_id TEXT NOT NULL REFERENCES order_items (id) ON DELETE CASCADE,
    promotion_id TEXT,
    code TEXT,
    amount INTEGER NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS order_adjustments (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    amount INTEGER NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS order_status_history (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    previous_status TEXT,
    note TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_status_history_order
    ON order_status_history (order_id, created_at);

CREATE TABLE IF NOT EXISTS order_fulfillments (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'pending',
    tracking_numbers TEXT NOT NULL DEFAULT '[]',
    shipped_at INTEGER,
    delivered_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_fulfillments_order
    ON order_fulfillments (order_id);

CREATE TABLE IF NOT EXISTS order_refunds (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    payment_id TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    reason TEXT,
    provider_refund_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_refunds_order ON order_refunds (order_id);
