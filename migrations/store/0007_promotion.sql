-- Promotions, coupons, deterministic rules and usage accounting.

CREATE TABLE IF NOT EXISTS promotions (
    id TEXT PRIMARY KEY,
    code TEXT UNIQUE,
    title TEXT NOT NULL,
    type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft',
    is_automatic INTEGER NOT NULL DEFAULT 0,
    starts_at INTEGER,
    ends_at INTEGER,
    usage_limit INTEGER,
    usage_count INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promotions_status ON promotions (status, starts_at, ends_at);

CREATE TABLE IF NOT EXISTS promotion_rules (
    id TEXT PRIMARY KEY,
    promotion_id TEXT NOT NULL REFERENCES promotions (id) ON DELETE CASCADE,
    attribute TEXT NOT NULL,
    operator TEXT NOT NULL DEFAULT 'eq',
    value TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promotion_rules_promotion
    ON promotion_rules (promotion_id);

CREATE TABLE IF NOT EXISTS coupons (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    promotion_id TEXT NOT NULL REFERENCES promotions (id) ON DELETE CASCADE,
    usage_limit INTEGER,
    usage_count INTEGER NOT NULL DEFAULT 0,
    starts_at INTEGER,
    ends_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_coupons_promotion ON coupons (promotion_id);

CREATE TABLE IF NOT EXISTS promotion_usage (
    id TEXT PRIMARY KEY,
    promotion_id TEXT NOT NULL,
    coupon_id TEXT,
    customer_id TEXT,
    order_id TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promotion_usage_promotion
    ON promotion_usage (promotion_id);
CREATE INDEX IF NOT EXISTS idx_promotion_usage_customer
    ON promotion_usage (customer_id);

CREATE TABLE IF NOT EXISTS promotion_applications (
    id TEXT PRIMARY KEY,
    promotion_id TEXT NOT NULL,
    cart_id TEXT,
    order_id TEXT,
    code TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_promotion_applications_cart
    ON promotion_applications (cart_id);
