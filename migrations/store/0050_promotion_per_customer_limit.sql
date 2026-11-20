-- Each customer may redeem a promotion at most `per_customer_limit` times.
-- A NULL value keeps the promotion unlimited per customer.
ALTER TABLE promotions ADD COLUMN per_customer_limit INTEGER;
