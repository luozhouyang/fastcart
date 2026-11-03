-- Provider-independent payments, attempts, webhooks and refunds.

CREATE TABLE IF NOT EXISTS payment_sessions (
    id TEXT PRIMARY KEY,
    cart_id TEXT,
    checkout_id TEXT,
    order_id TEXT,
    provider TEXT NOT NULL,
    provider_session_id TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    data TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payment_sessions_checkout
    ON payment_sessions (checkout_id);
CREATE INDEX IF NOT EXISTS idx_payment_sessions_order
    ON payment_sessions (order_id);

CREATE TABLE IF NOT EXISTS payment_attempts (
    id TEXT PRIMARY KEY,
    payment_session_id TEXT NOT NULL REFERENCES payment_sessions (id) ON DELETE CASCADE,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    provider_reference TEXT,
    error TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_payment_attempts_session
    ON payment_attempts (payment_session_id);

CREATE TABLE IF NOT EXISTS payments (
    id TEXT PRIMARY KEY,
    order_id TEXT,
    provider TEXT NOT NULL,
    provider_payment_id TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    captured_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_payments_provider_id
    ON payments (provider, provider_payment_id)
    WHERE provider_payment_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_payments_order ON payments (order_id);

CREATE TABLE IF NOT EXISTS payment_webhooks (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    provider_event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'received',
    attempts INTEGER NOT NULL DEFAULT 0,
    processed_at INTEGER,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    UNIQUE (provider, provider_event_id)
);

CREATE TABLE IF NOT EXISTS refunds (
    id TEXT PRIMARY KEY,
    payment_id TEXT REFERENCES payments (id) ON DELETE SET NULL,
    order_id TEXT,
    amount INTEGER NOT NULL,
    currency_code TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    provider_refund_id TEXT,
    reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_refunds_order ON refunds (order_id);
