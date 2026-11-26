-- Central translation storage for storefront content.
--
-- One row per (resource_type, resource_id, locale) holds a JSON object of
-- translated fields. Missing or null fields fall back to the source content or
-- to the store's default locale; an empty string is an explicit empty
-- translation.
--
-- Allowed resource_type values: product, category, collection, page,
-- navigation. Lookups are always by resource and locale, so the unique index
-- also serves as the primary read path.

CREATE TABLE IF NOT EXISTS translations (
    id TEXT PRIMARY KEY,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    locale TEXT NOT NULL,
    fields_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_translations_resource_locale
    ON translations (resource_type, resource_id, locale);

CREATE INDEX IF NOT EXISTS idx_translations_locale
    ON translations (locale);
