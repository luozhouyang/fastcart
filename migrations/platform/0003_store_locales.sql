-- Store locales: the display languages a store publishes.
--
-- `stores.locale` remains the source locale of the base content. One row per
-- store here is the default display locale, enforced by a partial unique
-- index so a store can never have two defaults.

CREATE TABLE IF NOT EXISTS store_locales (
    store_id TEXT NOT NULL REFERENCES stores (id) ON DELETE CASCADE,
    locale TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    label TEXT,
    direction TEXT NOT NULL DEFAULT 'ltr',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (store_id, locale)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_store_locales_default
    ON store_locales (store_id) WHERE is_default = 1;
