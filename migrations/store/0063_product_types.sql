-- Product types: a coarse, store-managed taxonomy assigned to products.
--
-- A product references at most one type. SQLite cannot attach an
-- `ON DELETE SET NULL` action to an existing table without rebuilding it, so
-- the catalog service clears `products.product_type_id` inside the same
-- transaction that deletes a type. The `REFERENCES` clause is added without a
-- default, which SQLite permits and enforces like any other foreign key.

CREATE TABLE product_types (
    id TEXT PRIMARY KEY,
    value TEXT NOT NULL UNIQUE,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

ALTER TABLE products ADD COLUMN product_type_id TEXT REFERENCES product_types(id);

CREATE INDEX idx_products_type ON products(product_type_id);
