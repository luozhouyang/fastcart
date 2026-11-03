-- Customers, addresses, groups, credentials and store-scoped sessions.

CREATE TABLE IF NOT EXISTS customer_groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS customers (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    first_name TEXT,
    last_name TEXT,
    phone TEXT,
    has_account INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_customers_email ON customers (email);

CREATE TABLE IF NOT EXISTS customer_addresses (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    is_default_shipping INTEGER NOT NULL DEFAULT 0,
    is_default_billing INTEGER NOT NULL DEFAULT 0,
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
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_customer_addresses_customer
    ON customer_addresses (customer_id);

CREATE TABLE IF NOT EXISTS customer_credentials (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL UNIQUE REFERENCES customers (id) ON DELETE CASCADE,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS customer_sessions (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    last_used_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_customer_sessions_customer
    ON customer_sessions (customer_id);

CREATE TABLE IF NOT EXISTS customer_group_links (
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    group_id TEXT NOT NULL REFERENCES customer_groups (id) ON DELETE CASCADE,
    PRIMARY KEY (customer_id, group_id)
);
