-- Prepaid gift cards: an issued balance redeemed against future payments.

CREATE TABLE IF NOT EXISTS gift_cards (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    currency_code TEXT NOT NULL,
    initial_amount_minor INTEGER NOT NULL,
    balance_minor INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_gift_cards_code ON gift_cards (code);
CREATE INDEX IF NOT EXISTS idx_gift_cards_status ON gift_cards (status);
