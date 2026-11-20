-- Explicit ordering and opt-out stacking for promotions.
--
-- `priority` weighs a promotion in the discount sequence: higher values are
-- applied first, `created_at` and then `id` break ties. The default of 0 keeps
-- every row written before this migration in the historic `created_at, id`
-- order, so existing stores allocate discounts exactly as they did.
--
-- `is_stackable` 0 makes a promotion exclusive: once it produces a discount,
-- the promotions ordered after it are skipped for that batch. The default of 1
-- keeps every existing row stacking, again preserving the historic behaviour.
ALTER TABLE promotions ADD COLUMN priority INTEGER NOT NULL DEFAULT 0;
ALTER TABLE promotions ADD COLUMN is_stackable INTEGER NOT NULL DEFAULT 1;
