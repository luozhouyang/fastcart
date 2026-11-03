-- Price sets: a Medusa-style container for every price of a variant.

CREATE TABLE IF NOT EXISTS price_sets (
    id TEXT PRIMARY KEY,
    title TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS product_variant_price_set (
    variant_id TEXT PRIMARY KEY,
    price_set_id TEXT NOT NULL REFERENCES price_sets (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_product_variant_price_set_set
    ON product_variant_price_set (price_set_id);

ALTER TABLE prices
    ADD COLUMN price_set_id TEXT REFERENCES price_sets (id) ON DELETE CASCADE;

CREATE INDEX IF NOT EXISTS idx_prices_price_set ON prices (price_set_id);

-- Backfill one price set per existing variant. The identifier is derived from
-- the variant identifier so the mapping can be written entirely in SQL. The
-- union includes variants that only appear in the prices table so that every
-- price can be linked without violating the foreign key.
INSERT INTO price_sets (id, title, metadata, created_at, updated_at)
SELECT 'ps_' || variants.variant_id,
       COALESCE(
           (SELECT pv.title FROM product_variants pv WHERE pv.id = variants.variant_id),
           'Price set'
       ),
       '{}',
       CAST(strftime('%s', 'now') AS INTEGER),
       CAST(strftime('%s', 'now') AS INTEGER)
FROM (
    SELECT id AS variant_id FROM product_variants
    UNION
    SELECT DISTINCT variant_id FROM prices
) AS variants
WHERE NOT EXISTS (
    SELECT 1 FROM price_sets ps WHERE ps.id = 'ps_' || variants.variant_id
);

INSERT INTO product_variant_price_set (variant_id, price_set_id)
SELECT variants.variant_id, 'ps_' || variants.variant_id
FROM (
    SELECT id AS variant_id FROM product_variants
    UNION
    SELECT DISTINCT variant_id FROM prices
) AS variants
WHERE NOT EXISTS (
    SELECT 1 FROM product_variant_price_set link WHERE link.variant_id = variants.variant_id
);

UPDATE prices
SET price_set_id = 'ps_' || variant_id
WHERE price_set_id IS NULL;
