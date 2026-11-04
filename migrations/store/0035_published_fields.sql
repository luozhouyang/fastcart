-- Snapshot every storefront-visible page field at publication time.
--
-- `published_body` (0034) isolates draft edits to the body. Routing and
-- presentation fields are similarly mutable drafts, so storefront reads must
-- serve the values captured by the last publish. Nullable columns keep legacy
-- rows readable: readers fall back to the live column when a snapshot value is
-- absent.

ALTER TABLE pages ADD COLUMN published_title TEXT;
ALTER TABLE pages ADD COLUMN published_handle TEXT;
ALTER TABLE pages ADD COLUMN published_page_type TEXT;
ALTER TABLE pages ADD COLUMN published_seo TEXT;
ALTER TABLE pages ADD COLUMN published_metadata TEXT;

UPDATE pages
SET published_title = title,
    published_handle = handle,
    published_page_type = page_type,
    published_seo = seo,
    published_metadata = metadata
WHERE status = 'published';
