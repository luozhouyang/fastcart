-- Attach the storefront region a cart/checkout was priced for.
--
-- IDs stay plain TEXT because carts and checkout sessions live in the cart and
-- checkout modules, which must not join across module-owned tables (regions is
-- owned by the store module).

ALTER TABLE carts ADD COLUMN region_id TEXT;

ALTER TABLE checkout_sessions ADD COLUMN region_id TEXT;

CREATE INDEX IF NOT EXISTS idx_carts_region
    ON carts (region_id);

CREATE INDEX IF NOT EXISTS idx_checkout_sessions_region
    ON checkout_sessions (region_id);
