-- Where a promotion's discount applies.
--
-- `items` keeps the historic behaviour: percentages and fixed amounts discount
-- the eligible line items. `order` applies them to the item subtotal plus
-- shipping, `shipping` to shipping alone. The default backfills every existing
-- row with the historic behaviour, so a store that never configures a target
-- allocates discounts exactly as it did.
ALTER TABLE promotions ADD COLUMN target TEXT NOT NULL DEFAULT 'items';
