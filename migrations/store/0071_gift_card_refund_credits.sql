-- Money a refund put back on gift cards before crediting the payment provider.
--
-- A checkout paid by gift card records the card's share in
-- `payment_gift_card_redemptions`. Refunding the resulting payment must restore
-- that money to the card first and send only the remainder to the provider, so
-- one row here records every redemption a refund credited. The unique
-- `(refund_id, redemption_id)` index makes a retried refund idempotent at the
-- ledger level: the same refund can never credit the same redemption twice,
-- while separate refunds stay individually auditable.

CREATE TABLE IF NOT EXISTS gift_card_refund_credits (
    id TEXT PRIMARY KEY,
    refund_id TEXT NOT NULL,
    redemption_id TEXT NOT NULL,
    gift_card_id TEXT NOT NULL,
    amount_minor INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_gift_card_refund_credits_refund_redemption
    ON gift_card_refund_credits (refund_id, redemption_id);
CREATE INDEX IF NOT EXISTS idx_gift_card_refund_credits_redemption
    ON gift_card_refund_credits (redemption_id);
