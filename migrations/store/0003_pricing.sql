-- Pricing: price lists, rules and variant prices.

CREATE TABLE IF NOT EXISTS price_lists (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT,
    type TEXT NOT NULL DEFAULT 'sale',
    status TEXT NOT NULL DEFAULT 'draft',
    starts_at INTEGER,
    ends_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS price_list_rules (
    id TEXT PRIMARY KEY,
    price_list_id TEXT NOT NULL REFERENCES price_lists (id) ON DELETE CASCADE,
    attribute TEXT NOT NULL,
    operator TEXT NOT NULL DEFAULT 'eq',
    value TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_price_list_rules_list
    ON price_list_rules (price_list_id);

CREATE TABLE IF NOT EXISTS prices (
    id TEXT PRIMARY KEY,
    price_list_id TEXT REFERENCES price_lists (id) ON DELETE CASCADE,
    variant_id TEXT NOT NULL,
    currency_code TEXT NOT NULL,
    amount INTEGER NOT NULL,
    min_quantity INTEGER NOT NULL DEFAULT 1,
    max_quantity INTEGER,
    customer_group_id TEXT,
    sales_channel_id TEXT,
    starts_at INTEGER,
    ends_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prices_variant_currency
    ON prices (variant_id, currency_code);
CREATE INDEX IF NOT EXISTS idx_prices_list ON prices (price_list_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_prices_unique_scope
    ON prices (
        variant_id,
        currency_code,
        IFNULL(price_list_id, ''),
        IFNULL(customer_group_id, ''),
        IFNULL(sales_channel_id, ''),
        min_quantity
    );

CREATE TABLE IF NOT EXISTS price_rules (
    id TEXT PRIMARY KEY,
    price_id TEXT REFERENCES prices (id) ON DELETE CASCADE,
    price_list_id TEXT REFERENCES price_lists (id) ON DELETE CASCADE,
    attribute TEXT NOT NULL,
    operator TEXT NOT NULL DEFAULT 'eq',
    value TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_price_rules_price ON price_rules (price_id);
