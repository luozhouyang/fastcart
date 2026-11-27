-- Gift cards as a way to pay for a cart, end to end.
--
-- `cart_gift_cards` is owned by the cart module: it records the amount a card
-- carried when the customer applied it. The card itself, and therefore the
-- live balance, stays in `gift_cards` (owned by the payment module) and is read
-- at quote time, so a card spent on another cart cannot be oversold here.
--
-- `payment_gift_card_redemptions` is owned by the payment module: one row per
-- card and checkout records the money the checkout actually took from the
-- card. It is written in the same transaction as the guarded balance decrement
-- (so a retried completion never spends a card twice), names the order once it
-- is placed, and keeps the redemption queryable from the admin.

CREATE TABLE IF NOT EXISTS cart_gift_cards (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL,
    gift_card_id TEXT NOT NULL,
    -- Masked redemption code: the full code is a bearer credential and stays
    -- in the payment-owned `gift_cards` table.
    masked_code TEXT NOT NULL,
    amount_minor INTEGER NOT NULL,
    -- Card balance read by the workflow when the cart was last priced. The
    -- cart never reads the payment-owned card itself; the workflow refreshes
    -- this snapshot on every recalculation.
    balance_minor INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_cart_gift_cards_cart_card
    ON cart_gift_cards (cart_id, gift_card_id);
CREATE INDEX IF NOT EXISTS idx_cart_gift_cards_cart
    ON cart_gift_cards (cart_id);

CREATE TABLE IF NOT EXISTS payment_gift_card_redemptions (
    id TEXT PRIMARY KEY,
    gift_card_id TEXT NOT NULL,
    checkout_id TEXT NOT NULL,
    order_id TEXT,
    amount_minor INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'redeemed',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    reversed_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_payment_gift_card_redemptions_checkout_card
    ON payment_gift_card_redemptions (checkout_id, gift_card_id);
CREATE INDEX IF NOT EXISTS idx_payment_gift_card_redemptions_order
    ON payment_gift_card_redemptions (order_id);
CREATE INDEX IF NOT EXISTS idx_payment_gift_card_redemptions_card
    ON payment_gift_card_redemptions (gift_card_id);
