-- Added lines keep the tax-inclusive flag they were authored with, so a draft
-- converted after an edit prices its lines the same way a checkout would.
ALTER TABLE order_edit_items ADD COLUMN is_tax_inclusive INTEGER NOT NULL DEFAULT 0;
