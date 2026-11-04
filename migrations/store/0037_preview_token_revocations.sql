-- Early revocation for signed draft preview tokens.
--
-- Preview tokens are stateless HMAC credentials valid for 15 minutes. An
-- administrator can revoke one token before expiry by recording its `jti`
-- here. Expired rows are harmless and may be pruned opportunistically.

CREATE TABLE IF NOT EXISTS preview_token_revocations (
    jti TEXT PRIMARY KEY,
    page_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_preview_token_revocations_expiry
    ON preview_token_revocations (expires_at);
