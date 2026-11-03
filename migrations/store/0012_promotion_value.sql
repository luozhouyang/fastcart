-- Promotion monetary value. Percentages are stored in basis points.
ALTER TABLE promotions ADD COLUMN value INTEGER NOT NULL DEFAULT 0;
