-- Drafts and published snapshots share one storefront handle namespace.
--
-- Storefront routing resolves a page by COALESCE(published_handle, handle)
-- (see ContentService::get_published_page_by_handle), so a draft handle and
-- another page's published snapshot can otherwise resolve to the same URL.
-- Enforce uniqueness of the effective storefront handle for non-deleted pages.
--
-- No data is deduplicated here: this platform is pre-release and silently
-- rewriting or deleting pages would be worse than a failed migration. If a
-- database already contains duplicate effective handles (possible only for
-- rows written before this index), resolve them manually before upgrading;
-- the unique index will otherwise refuse to be created.
CREATE UNIQUE INDEX IF NOT EXISTS pages_effective_handle_unique
    ON pages (COALESCE(published_handle, handle))
    WHERE deleted_at IS NULL;
