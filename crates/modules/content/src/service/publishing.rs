//! Page publishing, archiving and version restore.

use super::*;

impl ContentService {
    /// Publishes a page and snapshots its current body as a new version.
    ///
    /// The stored body is validated against the active theme editor manifest
    /// before publishing, and the live title, handle, page type, body, SEO and
    /// metadata are copied into their `published_*` columns, so storefront
    /// routing and presentation only change on publish. The revision is bumped
    /// so stale writers conflict. Publishing an archived page is a conflict.
    /// Re-publishing a page whose published snapshot already equals every live
    /// field is idempotent and leaves both the revision and snapshots
    /// untouched.
    pub async fn publish_page(&self, id: &PageId) -> Result<Page, AppError> {
        self.publish_page_as(id, None, PageRevisionOrigin::Manual)
            .await
    }
}

impl ContentService {
    /// Publishes a page, recording the actor and revision origin.
    ///
    /// This is the attributed form of [`Self::publish_page`]. The `created_by`
    /// value and `origin` are stored on the published snapshot.
    pub async fn publish_page_as(
        &self,
        id: &PageId,
        created_by: Option<&str>,
        origin: PageRevisionOrigin,
    ) -> Result<Page, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))?;

        if row.status == PageStatus::Archived.as_str() {
            return Err(AppError::conflict("cannot publish an archived page"));
        }

        self.validate_body(&parse_json(&row.body))?;

        if row.status == PageStatus::Published.as_str() && row.published_matches_live() {
            tx.commit().await.map_err(database_error)?;
            return Ok(row.into_page());
        }

        let now = now_timestamp();
        let next_version = next_page_version(&mut tx, id).await?;

        sqlx::query(
            "UPDATE pages SET status = 'published', published_body = body, \
             published_title = title, published_handle = handle, \
             published_page_type = page_type, published_seo = seo, \
             published_metadata = metadata, published_at = ?, updated_at = ?, \
             revision = revision + 1 WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            unique_conflict(error, "published handle is already used by another page")
        })?;

        insert_page_version(
            &mut tx,
            id,
            next_version,
            &row.body,
            created_by,
            PageVersionKind::Published,
            origin,
            now,
        )
        .await?;

        self.append_audit(
            &mut tx,
            "page.published",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": row.revision.saturating_add(1),
                "status": PageStatus::Published.as_str(),
                "origin": origin.as_str(),
                "kind": "published",
            }),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get_page(id).await
    }
}

impl ContentService {
    /// Moves a published page back to draft.
    ///
    /// Drafts are returned unchanged, while archived pages are a conflict. The
    /// last published body is retained for auditing and re-publication.
    pub async fn unpublish_page(&self, id: &PageId) -> Result<Page, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))?;
        if row.status == PageStatus::Archived.as_str() {
            return Err(AppError::conflict("cannot unpublish an archived page"));
        }
        if row.status != PageStatus::Published.as_str() {
            tx.commit().await.map_err(database_error)?;
            return Ok(row.into_page());
        }
        sqlx::query(
            "UPDATE pages SET status = 'draft', published_at = NULL, updated_at = ?, \
             revision = revision + 1 WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "page.unpublished",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": row.revision.saturating_add(1),
                "status": PageStatus::Draft.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.get_page(id).await
    }
}

impl ContentService {
    /// Archives a page, removing it from the storefront.
    ///
    /// Archiving an already archived page is idempotent and does not bump the
    /// revision. The last published body is retained.
    pub async fn archive_page(&self, id: &PageId) -> Result<Page, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))?;
        if row.status == PageStatus::Archived.as_str() {
            tx.commit().await.map_err(database_error)?;
            return Ok(row.into_page());
        }
        sqlx::query(
            "UPDATE pages SET status = 'archived', published_at = NULL, updated_at = ?, \
             revision = revision + 1 WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "page.archived",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": row.revision.saturating_add(1),
                "status": PageStatus::Archived.as_str(),
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.get_page(id).await
    }
}

impl ContentService {
    /// Lists a page's body snapshots newest first with pagination.
    pub async fn list_page_versions(
        &self,
        page_id: &PageId,
        pagination: Pagination,
    ) -> Result<PageResult<PageVersion>, AppError> {
        self.fetch_page_row(page_id, false).await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM page_versions WHERE page_id = ?")
            .bind(page_id.as_str())
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query_as::<_, PageVersionRow>(
            "SELECT id, page_id, version, body, created_by, kind, origin, created_at \
             FROM page_versions WHERE page_id = ? ORDER BY version DESC LIMIT ? OFFSET ?",
        )
        .bind(page_id.as_str())
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let data = rows.into_iter().map(PageVersionRow::into_version).collect();
        Ok(PageResult::new(data, count, pagination))
    }
}

impl ContentService {
    /// Restores a page body from a stored version and snapshots the restore.
    ///
    /// The restored body becomes the draft body; publishing remains an
    /// explicit separate step. The source version row is left untouched. When
    /// `base_revision` is supplied it must match the current revision, so a
    /// stale editor cannot overwrite a newer change.
    pub async fn restore_version(
        &self,
        page_id: &PageId,
        version: i64,
        base_revision: Option<i64>,
    ) -> Result<Page, AppError> {
        self.restore_version_as(
            page_id,
            version,
            base_revision,
            None,
            PageRevisionOrigin::Manual,
        )
        .await
    }
}

impl ContentService {
    /// Restores a page body, recording the actor and revision origin.
    ///
    /// Only the draft body is replaced; `published_body` and the live
    /// storefront content are never modified by a restore. The restored body is
    /// snapshotted as a draft with the supplied attribution.
    pub async fn restore_version_as(
        &self,
        page_id: &PageId,
        version: i64,
        base_revision: Option<i64>,
        created_by: Option<&str>,
        origin: PageRevisionOrigin,
    ) -> Result<Page, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let existing = sqlx::query_as::<_, PageRow>(&sql)
            .bind(page_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", page_id))?;

        if let Some(base_revision) = base_revision
            && base_revision != existing.revision
        {
            return Err(revision_conflict(existing.revision));
        }

        let version_row = sqlx::query_as::<_, PageVersionRow>(
            "SELECT id, page_id, version, body, created_by, kind, origin, created_at \
             FROM page_versions WHERE page_id = ? AND version = ?",
        )
        .bind(page_id.as_str())
        .bind(version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("page version", format!("{page_id}@{version}")))?;

        let now = now_timestamp();
        self.validate_body(&parse_json(&version_row.body))?;
        let next_version = next_page_version(&mut tx, page_id).await?;

        sqlx::query(
            "UPDATE pages SET body = ?, updated_at = ?, revision = revision + 1 WHERE id = ?",
        )
        .bind(&version_row.body)
        .bind(now)
        .bind(page_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        insert_page_version(
            &mut tx,
            page_id,
            next_version,
            &version_row.body,
            created_by,
            PageVersionKind::Draft,
            origin,
            now,
        )
        .await?;

        self.append_audit(
            &mut tx,
            "page.restored",
            "page",
            page_id.as_str(),
            serde_json::json!({
                "version": version,
                "revision": existing.revision.saturating_add(1),
                "origin": origin.as_str(),
            }),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get_page(page_id).await
    }
}
