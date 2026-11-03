-- Catalog: products, variants, options, media and organization.

CREATE TABLE IF NOT EXISTS products (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    subtitle TEXT,
    description TEXT,
    handle TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'draft',
    thumbnail TEXT,
    weight REAL,
    length REAL,
    height REAL,
    width REAL,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_products_status ON products (status);
CREATE INDEX IF NOT EXISTS idx_products_handle ON products (handle);

CREATE TABLE IF NOT EXISTS product_variants (
    id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    sku TEXT UNIQUE,
    barcode TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    weight REAL,
    length REAL,
    height REAL,
    width REAL,
    manage_inventory INTEGER NOT NULL DEFAULT 1,
    allow_backorder INTEGER NOT NULL DEFAULT 0,
    tax_category_id TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_product_variants_product ON product_variants (product_id);
CREATE INDEX IF NOT EXISTS idx_product_variants_sku ON product_variants (sku);

CREATE TABLE IF NOT EXISTS product_options (
    id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS product_option_values (
    id TEXT PRIMARY KEY,
    product_option_id TEXT NOT NULL REFERENCES product_options (id) ON DELETE CASCADE,
    value TEXT NOT NULL,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS product_variant_option_values (
    variant_id TEXT NOT NULL REFERENCES product_variants (id) ON DELETE CASCADE,
    option_value_id TEXT NOT NULL REFERENCES product_option_values (id) ON DELETE CASCADE,
    PRIMARY KEY (variant_id, option_value_id)
);

CREATE TABLE IF NOT EXISTS product_images (
    id TEXT PRIMARY KEY,
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    variant_id TEXT REFERENCES product_variants (id) ON DELETE SET NULL,
    url TEXT NOT NULL,
    alt TEXT,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_product_images_product ON product_images (product_id);

CREATE TABLE IF NOT EXISTS categories (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES categories (id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    description TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    is_internal INTEGER NOT NULL DEFAULT 0,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_categories_parent ON categories (parent_id);

CREATE TABLE IF NOT EXISTS collections (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    description TEXT,
    is_published INTEGER NOT NULL DEFAULT 0,
    rank INTEGER NOT NULL DEFAULT 0,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY,
    value TEXT NOT NULL UNIQUE,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS product_category_links (
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    category_id TEXT NOT NULL REFERENCES categories (id) ON DELETE CASCADE,
    rank INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (product_id, category_id)
);

CREATE INDEX IF NOT EXISTS idx_product_category_links_category
    ON product_category_links (category_id);

CREATE TABLE IF NOT EXISTS product_collection_links (
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
    rank INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (product_id, collection_id)
);

CREATE INDEX IF NOT EXISTS idx_product_collection_links_collection
    ON product_collection_links (collection_id);

CREATE TABLE IF NOT EXISTS product_tag_links (
    product_id TEXT NOT NULL REFERENCES products (id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    PRIMARY KEY (product_id, tag_id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS product_search USING fts5 (
    product_id UNINDEXED,
    title,
    description,
    sku,
    handle UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
