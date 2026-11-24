-- Order edits settle their own price difference and move their own stock.
--
-- A confirmed edit already rewrites `order_items` and derives the order totals
-- from those rows. What it did not do is move inventory, or record the money
-- difference the edit leaves between the order and what the customer paid.
--
-- `order_edits.difference_total` stores the change the confirmation actually
-- made to `orders.total`, measured against the total that was current when the
-- confirmation transaction began. Collecting or refunding that difference is
-- then the same idempotent admin call the claim and exchange flows use. It is
-- NULL until the edit is confirmed.
--
-- The partial unique index makes an `order_edit` movement reference unique per
-- edit line. The reference is derived from the order, the edit and the edit
-- line, so a replayed confirmation can never move stock twice.

ALTER TABLE order_edits ADD COLUMN difference_total INTEGER;

-- An added line may be a digital product (a gift card, say), so the edit line
-- has to remember whether it takes stock. Existing lines inherit the value
-- from the `order_items` row they rewrite; an added line defaults to shipping
-- because that is what `order_items.requires_shipping` defaults to.
ALTER TABLE order_edit_items ADD COLUMN requires_shipping INTEGER NOT NULL DEFAULT 1;

CREATE UNIQUE INDEX IF NOT EXISTS idx_inventory_movements_order_edit
    ON inventory_movements (reference)
    WHERE reason = 'order_edit' AND reference IS NOT NULL;
