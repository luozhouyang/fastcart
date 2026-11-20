-- Signed price differences settled when a claim or exchange is accepted.
--
-- `difference_total` is the sum over the record's replacement lines of
-- `(replacement unit price - replaced unit price) * quantity`. A positive total
-- is owed by the customer, a negative one is owed back to them. NULL means the
-- difference has not been settled yet; 0 means the replacement and the line it
-- replaces were priced alike and no order adjustment was written.

ALTER TABLE order_claims ADD COLUMN difference_total INTEGER;
ALTER TABLE order_exchanges ADD COLUMN difference_total INTEGER;
