-- Drafts and published snapshots share one storefront handle namespace.
--
-- Storefront routing resolves a page by COALESCE(published_handle, handle)
-- (see ContentService::get_published_page_by_handle), so a draft handle and
-- another page's published snapshot can otherwise resolve to the same URL.
-- Enforce uniqueness of the effective storefront handle for non-deleted pages.
--
-- Upgrade repair: a legacy draft (never published, so `published_handle` is
-- NULL) may hold a live handle that another page already publishes under. Such
-- a draft has no storefront route of its own, so we rename its live handle with
-- its unique page id instead of failing the migration or touching a published
-- route. Two published pages sharing one published handle cannot be repaired
-- without guessing which route is correct; the unique index below will refuse
-- to build until an operator resolves that conflict.
UPDATE pages AS draft
SET handle = draft.handle || '-legacy-' || draft.id
WHERE draft.deleted_at IS NULL
  AND draft.published_handle IS NULL
  AND EXISTS (
      SELECT 1
      FROM pages AS published
      WHERE published.deleted_at IS NULL
        AND published.published_handle = draft.handle
  );

CREATE UNIQUE INDEX IF NOT EXISTS pages_effective_handle_unique
    ON pages (COALESCE(published_handle, handle))
    WHERE deleted_at IS NULL;
