-- Fulfillment sets and provider-backed shipping options.

CREATE TABLE IF NOT EXISTS fulfillment_sets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'shipping',
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_fulfillment_sets_type
    ON fulfillment_sets (type);

ALTER TABLE shipping_options ADD COLUMN provider_id TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE shipping_options ADD COLUMN provider_data TEXT NOT NULL DEFAULT '{}';
ALTER TABLE shipping_options ADD COLUMN fulfillment_set_id TEXT
    REFERENCES fulfillment_sets (id) ON DELETE SET NULL;

INSERT INTO fulfillment_sets (id, name, type, metadata, created_at, updated_at)
VALUES (
    'fset_default',
    'Default fulfillment set',
    'shipping',
    '{}',
    CAST(strftime('%s', 'now') AS INTEGER),
    CAST(strftime('%s', 'now') AS INTEGER)
);

UPDATE shipping_options
SET fulfillment_set_id = 'fset_default'
WHERE fulfillment_set_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_shipping_options_provider
    ON shipping_options (provider_id);

CREATE INDEX IF NOT EXISTS idx_shipping_options_fulfillment_set
    ON shipping_options (fulfillment_set_id);
