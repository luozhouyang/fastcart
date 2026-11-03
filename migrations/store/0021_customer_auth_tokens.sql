-- Customer email verification and password-reset tokens.
--
-- Plaintext tokens are never persisted: only their SHA-256 digest is stored,
-- exactly like storefront session tokens.

ALTER TABLE customers ADD COLUMN email_verified_at INTEGER;

CREATE TABLE IF NOT EXISTS customer_auth_tokens (
    id TEXT PRIMARY KEY,
    customer_id TEXT NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    token_type TEXT NOT NULL CHECK (token_type IN ('email_verification', 'password_reset')),
    expires_at INTEGER NOT NULL,
    used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_customer_auth_tokens_lookup
    ON customer_auth_tokens (customer_id, token_type, expires_at);
