-- Links after-sales records to the order refund that settled them.
--
-- Returns already carry `refund_id` from 0018. Claims gain the same link so a
-- refund accepted for a claim can be found and replayed without refunding the
-- customer twice.

ALTER TABLE order_claims ADD COLUMN refund_id TEXT;

CREATE INDEX IF NOT EXISTS idx_order_claims_refund
    ON order_claims (refund_id);
