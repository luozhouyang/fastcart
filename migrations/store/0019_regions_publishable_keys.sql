-- Medusa-style regions and storefront publishable API keys.

CREATE TABLE IF NOT EXISTS regions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    currency_code TEXT NOT NULL,
    countries TEXT NOT NULL DEFAULT '[]',
    tax_region_id TEXT,
    payment_providers TEXT NOT NULL DEFAULT '[]',
    is_default INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_regions_default
    ON regions (is_default) WHERE is_default = 1;

CREATE TABLE IF NOT EXISTS publishable_api_keys (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    token_prefix TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    sales_channel_ids TEXT NOT NULL DEFAULT '[]',
    revoked_at INTEGER,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_publishable_api_keys_hash
    ON publishable_api_keys (token_hash);
