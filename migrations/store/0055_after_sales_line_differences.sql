-- Per-line price differences, settled with the record they belong to.
--
-- `difference` is `(replacement unit price - replaced unit price) * quantity`
-- for one replacement line, in the same sign convention as the record's
-- `difference_total`: positive is owed by the customer, negative to them. It is
-- written when the record is accepted, next to the total it adds up to, so a
-- store can see which replacement moved the money.
--
-- NULL means the line settled nothing: the record is not accepted yet, the line
-- is an inbound line, or the caller resolved no price for its replacement.

ALTER TABLE order_exchange_items ADD COLUMN difference INTEGER;
ALTER TABLE order_claim_items ADD COLUMN difference INTEGER;
