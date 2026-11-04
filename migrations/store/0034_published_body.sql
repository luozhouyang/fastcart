-- Separate live page content from working draft edits.
--
-- `body` is the mutable draft. `published_body` stores the last body served to
-- storefronts; it is only updated when a page is published. Existing published
-- pages are backfilled so their live content keeps rendering unchanged.

ALTER TABLE pages ADD COLUMN published_body TEXT;

UPDATE pages SET published_body = body WHERE status = 'published';
