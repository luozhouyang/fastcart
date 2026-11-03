-- Shipping and tax configuration.

CREATE TABLE IF NOT EXISTS shipping_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'default',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS shipping_options (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES shipping_profiles (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    price_type TEXT NOT NULL DEFAULT 'flat',
    amount INTEGER NOT NULL DEFAULT 0,
    currency_code TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_shipping_options_profile
    ON shipping_options (profile_id);

CREATE TABLE IF NOT EXISTS shipping_rates (
    id TEXT PRIMARY KEY,
    shipping_option_id TEXT NOT NULL REFERENCES shipping_options (id) ON DELETE CASCADE,
    country_code TEXT,
    province_code TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    min_subtotal INTEGER,
    max_subtotal INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_shipping_rates_option
    ON shipping_rates (shipping_option_id);

CREATE TABLE IF NOT EXISTS shipping_option_locations (
    shipping_option_id TEXT NOT NULL REFERENCES shipping_options (id) ON DELETE CASCADE,
    location_id TEXT NOT NULL,
    PRIMARY KEY (shipping_option_id, location_id)
);

CREATE TABLE IF NOT EXISTS tax_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    code TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tax_regions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    country_code TEXT NOT NULL,
    province_code TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (country_code, province_code)
);

CREATE TABLE IF NOT EXISTS tax_rates (
    id TEXT PRIMARY KEY,
    tax_region_id TEXT NOT NULL REFERENCES tax_regions (id) ON DELETE CASCADE,
    tax_category_id TEXT,
    name TEXT NOT NULL,
    rate_bps INTEGER NOT NULL,
    is_inclusive INTEGER NOT NULL DEFAULT 0,
    is_default INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tax_rates_region ON tax_rates (tax_region_id);
