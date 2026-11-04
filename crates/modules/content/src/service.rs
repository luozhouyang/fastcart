//! SQLite-backed application service for content pages and navigation.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::{NewAuditOutbox, StoreDatabase, append_audit_outbox, database_error};
use fastcart_kernel::{
    AppError, Metadata, NavigationId, PageId, PageResult, Pagination, now_timestamp,
    validate_handle, validate_length, validate_metadata,
};
use fastcart_storefront::{PageBody, StorefrontError, ThemeEditorManifest};
use serde_json::Value;
use sqlx::FromRow;

use crate::domain::{
    CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_PAGE_TYPE_LENGTH,
    MAX_REDIRECT_PATH_LENGTH, MAX_TITLE_LENGTH, MAX_URL_LENGTH, Navigation, NavigationItem,
    NavigationItemInput, NavigationTree, NavigationTreeItem, Page, PageRevisionOrigin, PageStatus,
    PageVersion, PageVersionKind, Redirect, UpdateNavigationInput, UpdatePageInput, generate_id,
    resolve_handle, validate_page_content, validate_page_title, validate_redirect_status,
};

const PAGE_COLUMNS: &str = "id, title, handle, status, page_type, body, published_body, \
     published_title, published_handle, published_page_type, published_seo, published_metadata, \
     seo, metadata, revision, published_at, created_at, updated_at, deleted_at";

const NAVIGATION_COLUMNS: &str = "id, name, handle, metadata, created_at, updated_at";

const NAVIGATION_ITEM_COLUMNS: &str = "id, navigation_id, parent_id, title, url, rank, metadata";

/// Application service for store pages, page versions, navigation and redirects.
#[derive(Clone, Debug)]
pub struct ContentService {
    db: StoreDatabase,
    editor_manifest: Option<ThemeEditorManifest>,
    audit_actor: Option<String>,
}

impl ContentService {
    /// Creates a service bound to one store database without a theme editor
    /// manifest. Page bodies are still structurally validated.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self {
            db,
            editor_manifest: None,
            audit_actor: None,
        }
    }

    /// Creates a service bound to one store database, optionally validating
    /// page bodies against the active theme editor manifest.
    #[must_use]
    pub fn with_editor_manifest(
        db: StoreDatabase,
        editor_manifest: Option<ThemeEditorManifest>,
    ) -> Self {
        Self {
            db,
            editor_manifest,
            audit_actor: None,
        }
    }

    /// Records content mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change, so a crash cannot separate them.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit_actor = actor_id;
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: Value,
    ) -> Result<(), AppError> {
        let Some(actor_id) = self.audit_actor.as_deref() else {
            return Ok(());
        };
        let entry = NewAuditOutbox {
            actor_id: Some(actor_id.to_owned()),
            store_id: Some(self.db.store_id().as_str().to_owned()),
            action: action.to_owned(),
            resource_type: Some(resource_type.to_owned()),
            resource_id: Some(resource_id.to_owned()),
            data,
        };
        append_audit_outbox(&mut **tx, &entry)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    /// Validates a page body structurally and, when a manifest is available,
    /// against the theme editor contract.
    fn validate_body(&self, body: &Value) -> Result<PageBody, AppError> {
        let parsed = PageBody::parse(body).map_err(storefront_error)?;
        if let Some(editor) = self.editor_manifest.as_ref() {
            parsed
                .validate_with_editor(editor)
                .map_err(storefront_error)?;
        }
        Ok(parsed)
    }

    /// Creates a page. The handle defaults to a slug of the title.
    pub async fn create_page(&self, input: CreatePageInput) -> Result<Page, AppError> {
        let CreatePageInput {
            title,
            handle,
            page_type,
            body,
            seo,
            metadata,
        } = input;

        validate_page_title(&title)?;
        let title = title.trim().to_owned();
        let handle = resolve_handle(&title, handle)?;
        let page_type = match page_type {
            Some(page_type) => {
                validate_length(&page_type, "page_type", MAX_PAGE_TYPE_LENGTH)?;
                page_type.trim().to_owned()
            }
            None => "custom".to_owned(),
        };
        validate_page_content(&body, &seo)?;
        self.validate_body(&body)?;
        validate_metadata(&metadata)?;
        self.ensure_page_handle_available(&handle, None).await?;

        let id = PageId::new();
        let now = now_timestamp();
        let body_json = serialize_json(&body, "body")?;
        let seo_json = serialize_json(&seo, "seo")?;
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO pages (id, title, handle, status, page_type, body, published_body, \
             published_title, published_handle, published_page_type, published_seo, \
             published_metadata, seo, metadata, revision, published_at, created_at, updated_at, \
             deleted_at) \
             VALUES (?, ?, ?, 'draft', ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?, 1, NULL, \
             ?, ?, NULL)",
        )
        .bind(id.as_str())
        .bind(&title)
        .bind(&handle)
        .bind(&page_type)
        .bind(&body_json)
        .bind(&seo_json)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "page handle already exists"))?;
        self.append_audit(
            &mut tx,
            "page.created",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": 1,
                "status": PageStatus::Draft.as_str(),
                "page_type": page_type,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_page(&id).await
    }

    /// Loads a page by identifier. Soft-deleted pages are not returned.
    pub async fn get_page(&self, id: &PageId) -> Result<Page, AppError> {
        self.fetch_page_row(id, false).await.map(PageRow::into_page)
    }

    /// Loads a live page by handle.
    pub async fn get_page_by_handle(&self, handle: &str) -> Result<Page, AppError> {
        let sql =
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE handle = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(handle)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", handle))?;
        Ok(row.into_page())
    }

    /// Loads a published page by handle for storefront rendering.
    ///
    /// Only live, published pages are returned. Routing uses the published
    /// handle, falling back to the live handle only for legacy rows whose
    /// published snapshot predates the column. The returned [`Page`] carries
    /// the published title, handle, page type, body, SEO and metadata, falling
    /// back per field to the draft value when that snapshot value is absent,
    /// and always clears [`Page::published_body`] so the draft can never leak to
    /// storefronts.
    pub async fn get_published_page_by_handle(&self, handle: &str) -> Result<Page, AppError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages \
             WHERE COALESCE(published_handle, handle) = ? AND status = 'published' \
               AND deleted_at IS NULL"
        );
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(handle)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", handle))?;
        Ok(row.into_published_page())
    }

    /// Loads a published page by identifier for storefront data binding.
    ///
    /// The projection matches [`Self::get_published_page_by_handle`]: every
    /// storefront-visible field comes from the published snapshot, with a
    /// per-field fallback to the live value for legacy rows, and
    /// [`Page::published_body`] is always cleared.
    pub async fn get_published_page(&self, id: &PageId) -> Result<Page, AppError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages \
             WHERE id = ? AND status = 'published' AND deleted_at IS NULL"
        );
        sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))
            .map(PageRow::into_published_page)
    }

    /// Loads the most recently published home page.
    ///
    /// A page whose published classifier is `home` or whose published handle is
    /// `home` is returned, newest publication first. Legacy rows fall back to
    /// the live classifier and handle. Because matching and ordering use the
    /// published snapshot, draft-only edits cannot change which home page is
    /// served. The projection matches [`Self::get_published_page_by_handle`], so
    /// the returned body is the published body and [`Page::published_body`] is
    /// cleared.
    pub async fn get_home_page(&self) -> Result<Option<Page>, AppError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages \
             WHERE status = 'published' AND deleted_at IS NULL \
               AND (COALESCE(published_page_type, page_type) = 'home' \
                    OR COALESCE(published_handle, handle) = 'home') \
             ORDER BY published_at DESC, id ASC \
             LIMIT 1"
        );
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(row.map(PageRow::into_published_page))
    }

    /// Lists pages with an optional status filter and pagination.
    pub async fn list_pages(
        &self,
        status: Option<PageStatus>,
        include_deleted: bool,
        pagination: Pagination,
    ) -> Result<PageResult<Page>, AppError> {
        let mut conditions = String::from(" WHERE 1 = 1");
        if !include_deleted {
            conditions.push_str(" AND deleted_at IS NULL");
        }
        if status.is_some() {
            conditions.push_str(" AND status = ?");
        }

        let count_sql = format!("SELECT COUNT(*) FROM pages{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(status) = status {
            count_query = count_query.bind(status.as_str());
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, PageRow>(&list_sql);
        if let Some(status) = status {
            list_query = list_query.bind(status.as_str());
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let data = rows.into_iter().map(PageRow::into_page).collect();
        Ok(PageResult::new(data, count, pagination))
    }

    /// Searches non-deleted pages by title or handle, newest first.
    ///
    /// The query is matched case-insensitively and LIKE wildcards in user
    /// input are escaped. An absent or blank query matches every page.
    pub async fn search_pages(
        &self,
        query: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Page>, AppError> {
        let pattern = like_pattern(query);
        let conditions = if pattern.is_some() {
            " WHERE deleted_at IS NULL AND (LOWER(title) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(handle) LIKE LOWER(?) ESCAPE '\\')"
        } else {
            " WHERE deleted_at IS NULL"
        };

        let count_sql = format!("SELECT COUNT(*) FROM pages{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, PageRow>(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let data = rows.into_iter().map(PageRow::into_page).collect();
        Ok(PageResult::new(data, count, pagination))
    }

    /// Updates page fields inside one immediate transaction.
    ///
    /// When `base_revision` is supplied it must match the stored revision;
    /// stale writers receive a conflict carrying the current revision. Content
    /// changes create a draft snapshot but do not alter published snapshots.
    pub async fn update_page(&self, id: &PageId, input: UpdatePageInput) -> Result<Page, AppError> {
        let UpdatePageInput {
            title,
            handle,
            page_type,
            body,
            seo,
            metadata,
            base_revision,
            origin,
            created_by,
        } = input;

        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let existing = sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))?;

        if let Some(base_revision) = base_revision
            && base_revision != existing.revision
        {
            return Err(revision_conflict(existing.revision));
        }

        let title = match title {
            Some(title) => {
                validate_page_title(&title)?;
                title.trim().to_owned()
            }
            None => existing.title.clone(),
        };
        let handle = match handle {
            Some(handle) => {
                validate_handle(&handle, "handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        let duplicate: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE handle = ? AND id <> ?")
                .bind(&handle)
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if duplicate > 0 {
            return Err(AppError::conflict(format!(
                "page handle already exists: {handle}"
            )));
        }
        let page_type = match page_type {
            Some(page_type) => {
                validate_length(&page_type, "page_type", MAX_PAGE_TYPE_LENGTH)?;
                page_type.trim().to_owned()
            }
            None => existing.page_type.clone(),
        };
        let body_supplied = body.is_some();
        let body = match body {
            Some(body) => body,
            None => parse_json(&existing.body),
        };
        let seo = match seo {
            Some(seo) => seo,
            None => parse_json(&existing.seo),
        };
        let metadata = match metadata {
            Some(metadata) => metadata,
            None => Metadata::from_json_str(&existing.metadata),
        };
        validate_page_content(&body, &seo)?;
        // A metadata-only update must remain possible after the active theme
        // changed; the stored body is validated when it is replaced or
        // published, not on every unrelated field edit.
        if body_supplied {
            self.validate_body(&body)?;
        }
        validate_metadata(&metadata)?;

        let body_json = serialize_json(&body, "body")?;
        let seo_json = serialize_json(&seo, "seo")?;
        let body_changed = body_json != existing.body;
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE pages SET title = ?, handle = ?, page_type = ?, body = ?, seo = ?, \
             metadata = ?, revision = revision + 1, updated_at = ? \
             WHERE id = ? AND revision = ? AND deleted_at IS NULL",
        )
        .bind(&title)
        .bind(&handle)
        .bind(&page_type)
        .bind(&body_json)
        .bind(&seo_json)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id.as_str())
        .bind(existing.revision)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "page handle already exists"))?;
        if result.rows_affected() == 0 {
            return Err(revision_conflict(existing.revision));
        }

        let revision_origin = origin.unwrap_or(PageRevisionOrigin::Manual);
        if body_changed {
            let next_version = next_page_version(&mut tx, id).await?;
            insert_page_version(
                &mut tx,
                id,
                next_version,
                &body_json,
                created_by.as_deref(),
                PageVersionKind::Draft,
                revision_origin,
                now,
            )
            .await?;
        }

        self.append_audit(
            &mut tx,
            "page.updated",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": existing.revision.saturating_add(1),
                "status": existing.status,
                "origin": revision_origin.as_str(),
            }),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get_page(id).await
    }

    /// Soft-deletes a page and hides it from storefront reads.
    ///
    /// The page row is read and updated inside one transaction, and the
    /// returned [`Page`] reflects the soft-deleted state with the revision and
    /// status that were actually removed. Callers use it for audit records so
    /// the logged revision cannot race a concurrent write.
    pub async fn delete_page(&self, id: &PageId) -> Result<Page, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL");
        let row = sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))?;

        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE pages SET deleted_at = ?, updated_at = ? WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("page", id));
        }
        self.append_audit(
            &mut tx,
            "page.deleted",
            "page",
            id.as_str(),
            serde_json::json!({
                "revision": row.revision,
                "status": row.status,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        let mut page = row.into_page();
        page.deleted_at = Some(now);
        page.updated_at = now;
        Ok(page)
    }

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

    /// Creates a navigation menu. The handle defaults to a slug of the name.
    pub async fn create_navigation(
        &self,
        input: CreateNavigationInput,
    ) -> Result<Navigation, AppError> {
        let CreateNavigationInput {
            name,
            handle,
            metadata,
        } = input;
        validate_length(&name, "name", MAX_TITLE_LENGTH)?;
        let name = name.trim().to_owned();
        let handle = resolve_handle(&name, handle)?;
        validate_metadata(&metadata)?;
        self.ensure_navigation_handle_available(&handle, None)
            .await?;

        let id = NavigationId::new();
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO navigations (id, name, handle, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&name)
        .bind(&handle)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation.created",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": handle, "name": name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_navigation(&id).await
    }

    /// Loads a navigation with its flat, rank-ordered items.
    pub async fn get_navigation(&self, id: &NavigationId) -> Result<Navigation, AppError> {
        let row = self.fetch_navigation_row(id).await?;
        let items = self.items_for_navigation(id).await?;
        Ok(row.into_navigation(items))
    }

    /// Loads a navigation by handle with its flat, rank-ordered items.
    pub async fn get_navigation_by_handle(&self, handle: &str) -> Result<Navigation, AppError> {
        let row = self.fetch_navigation_row_by_handle(handle).await?;
        let id = NavigationId::from_string(row.id.clone());
        let items = self.items_for_navigation(&id).await?;
        Ok(row.into_navigation(items))
    }

    /// Loads a navigation by handle with items nested by parent.
    pub async fn get_navigation_tree(&self, handle: &str) -> Result<NavigationTree, AppError> {
        let navigation = self.get_navigation_by_handle(handle).await?;
        Ok(navigation_tree_from(navigation))
    }

    /// Loads a navigation by identifier with items nested by parent.
    ///
    /// This performs a primary-key lookup before building the tree, avoiding a
    /// second handle lookup when the identifier is already known.
    pub async fn get_navigation_tree_by_id(
        &self,
        id: &NavigationId,
    ) -> Result<NavigationTree, AppError> {
        let navigation = self.get_navigation(id).await?;
        Ok(navigation_tree_from(navigation))
    }

    /// Lists navigation menus with their items.
    pub async fn list_navigations(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Navigation>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM navigations")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let sql = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigations ORDER BY created_at DESC, id DESC \
             LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let id = NavigationId::from_string(row.id.clone());
            let items = self.items_for_navigation(&id).await?;
            data.push(row.into_navigation(items));
        }
        Ok(PageResult::new(data, count, pagination))
    }

    /// Searches navigations by name or handle, newest first.
    ///
    /// The query is matched case-insensitively and LIKE wildcards in user
    /// input are escaped. An absent or blank query matches every navigation.
    pub async fn search_navigations(
        &self,
        query: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Navigation>, AppError> {
        let pattern = like_pattern(query);
        let conditions = if pattern.is_some() {
            " WHERE LOWER(name) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(handle) LIKE LOWER(?) ESCAPE '\\'"
        } else {
            ""
        };

        let count_sql = format!("SELECT COUNT(*) FROM navigations{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigations{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, NavigationRow>(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let id = NavigationId::from_string(row.id.clone());
            let items = self.items_for_navigation(&id).await?;
            data.push(row.into_navigation(items));
        }
        Ok(PageResult::new(data, count, pagination))
    }

    /// Updates navigation fields. Omitted fields keep their current value.
    pub async fn update_navigation(
        &self,
        id: &NavigationId,
        input: UpdateNavigationInput,
    ) -> Result<Navigation, AppError> {
        let existing = self.fetch_navigation_row(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_length(&name, "name", MAX_TITLE_LENGTH)?;
                name.trim().to_owned()
            }
            None => existing.name.clone(),
        };
        let handle = match input.handle {
            Some(handle) => {
                validate_handle(&handle, "handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        self.ensure_navigation_handle_available(&handle, Some(id))
            .await?;
        let metadata = match input.metadata {
            Some(metadata) => metadata,
            None => Metadata::from_json_str(&existing.metadata),
        };
        validate_metadata(&metadata)?;

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE navigations SET name = ?, handle = ?, metadata = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&handle)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation.updated",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": handle, "name": name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_navigation(id).await
    }

    /// Deletes a navigation and, through foreign keys, all of its items.
    pub async fn delete_navigation(&self, id: &NavigationId) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE id = ?");
        let row = sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", id))?;
        sqlx::query("DELETE FROM navigations WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation.deleted",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": row.handle, "name": row.name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Adds an item to a navigation.
    pub async fn add_navigation_item(
        &self,
        navigation_id: &NavigationId,
        input: NavigationItemInput,
    ) -> Result<NavigationItem, AppError> {
        self.fetch_navigation_row(navigation_id).await?;
        let title = self.validated_item_title(&input.title)?;
        let url = self.validated_item_url(&input.url)?;
        let rank = input.rank.unwrap_or(0);
        validate_rank(rank)?;
        if let Some(parent_id) = input.parent_id.as_deref() {
            self.ensure_parent_in_navigation(navigation_id, parent_id, None)
                .await?;
        }
        validate_metadata(&input.metadata)?;

        let id = generate_id("nitem");
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO navigation_items \
             (id, navigation_id, parent_id, title, url, rank, metadata) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(navigation_id.as_str())
        .bind(input.parent_id.as_deref())
        .bind(&title)
        .bind(&url)
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation_item.created",
            "navigation_item",
            &id,
            serde_json::json!({
                "navigation_id": navigation_id.as_str(),
                "title": title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.fetch_navigation_item(&id).await
    }

    /// Updates an existing navigation item.
    pub async fn update_navigation_item(
        &self,
        item_id: &str,
        input: NavigationItemInput,
    ) -> Result<NavigationItem, AppError> {
        let existing = self.fetch_navigation_item_row(item_id).await?;
        let navigation_id = NavigationId::from_string(existing.navigation_id.clone());
        let title = self.validated_item_title(&input.title)?;
        let url = self.validated_item_url(&input.url)?;
        let rank = input.rank.unwrap_or(existing.rank);
        validate_rank(rank)?;
        if let Some(parent_id) = input.parent_id.as_deref() {
            self.ensure_parent_in_navigation(&navigation_id, parent_id, Some(item_id))
                .await?;
        }
        validate_metadata(&input.metadata)?;

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE navigation_items SET parent_id = ?, title = ?, url = ?, rank = ?, \
             metadata = ? WHERE id = ?",
        )
        .bind(input.parent_id.as_deref())
        .bind(&title)
        .bind(&url)
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .bind(item_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation_item.updated",
            "navigation_item",
            item_id,
            serde_json::json!({
                "navigation_id": navigation_id.as_str(),
                "title": title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.fetch_navigation_item(item_id).await
    }

    /// Deletes an item and, through foreign keys, all descendant items.
    pub async fn remove_navigation_item(&self, item_id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE id = ?");
        let row = sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation item", item_id))?;
        sqlx::query("DELETE FROM navigation_items WHERE id = ?")
            .bind(item_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation_item.deleted",
            "navigation_item",
            item_id,
            serde_json::json!({
                "navigation_id": row.navigation_id,
                "title": row.title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Reorders navigation items by assigning ranks from the given order.
    pub async fn reorder_navigation_items(
        &self,
        navigation_id: &NavigationId,
        item_ids: &[String],
    ) -> Result<Vec<NavigationItem>, AppError> {
        self.fetch_navigation_row(navigation_id).await?;
        let mut seen = HashSet::new();
        for item_id in item_ids {
            if !seen.insert(item_id.as_str()) {
                return Err(AppError::validation(
                    "navigation item ids must be unique when reordering",
                ));
            }
        }

        let mut tx = self.db.begin().await?;
        for (index, item_id) in item_ids.iter().enumerate() {
            let owner: Option<String> =
                sqlx::query_scalar("SELECT navigation_id FROM navigation_items WHERE id = ?")
                    .bind(item_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            match owner {
                Some(owner) if owner == navigation_id.as_str() => {}
                Some(_) => {
                    return Err(AppError::validation(
                        "navigation item must belong to the same navigation",
                    ));
                }
                None => return Err(AppError::not_found("navigation item", item_id)),
            }
            let rank = i64::try_from(index)
                .map_err(|error| AppError::internal(format!("rank overflow: {error}")))?;
            sqlx::query("UPDATE navigation_items SET rank = ? WHERE id = ?")
                .bind(rank)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        }
        self.append_audit(
            &mut tx,
            "navigation.items_reordered",
            "navigation",
            navigation_id.as_str(),
            serde_json::json!({ "item_ids": item_ids }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.items_for_navigation(navigation_id).await
    }

    /// Creates a redirect.
    pub async fn create_redirect(&self, input: CreateRedirectInput) -> Result<Redirect, AppError> {
        let from_path = input.from_path.trim().to_owned();
        if !from_path.starts_with('/') {
            return Err(AppError::validation("from_path must start with '/'"));
        }
        validate_length(&from_path, "from_path", MAX_REDIRECT_PATH_LENGTH)?;
        let to_path = input.to_path.trim().to_owned();
        validate_length(&to_path, "to_path", MAX_REDIRECT_PATH_LENGTH)?;
        let status_code = input.status_code.unwrap_or(301);
        validate_redirect_status(status_code)?;

        let mut tx = self.db.begin().await?;
        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM redirects WHERE from_path = ?")
                .bind(&from_path)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if existing > 0 {
            return Err(AppError::conflict(format!(
                "redirect already exists for: {from_path}"
            )));
        }

        let id = generate_id("redirect");
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO redirects (id, from_path, to_path, status_code, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&from_path)
        .bind(&to_path)
        .bind(status_code)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "redirect.created",
            "redirect",
            &id,
            serde_json::json!({
                "from_path": from_path,
                "to_path": to_path,
                "status_code": status_code,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_redirect(&id).await
    }

    /// Loads a redirect by identifier.
    pub async fn get_redirect(&self, id: &str) -> Result<Redirect, AppError> {
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", id))?;
        Ok(row.into_redirect())
    }

    /// Loads a redirect by its source path.
    pub async fn get_redirect_by_path(&self, from_path: &str) -> Result<Redirect, AppError> {
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE from_path = ?",
        )
        .bind(from_path)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", from_path))?;
        Ok(row.into_redirect())
    }

    /// Lists redirects with pagination.
    pub async fn list_redirects(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Redirect>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM redirects")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let data = rows.into_iter().map(RedirectRow::into_redirect).collect();
        Ok(PageResult::new(data, count, pagination))
    }

    /// Deletes a redirect.
    pub async fn delete_redirect(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", id))?;
        sqlx::query("DELETE FROM redirects WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "redirect.deleted",
            "redirect",
            id,
            serde_json::json!({
                "from_path": row.from_path,
                "to_path": row.to_path,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn fetch_page_row(
        &self,
        id: &PageId,
        include_deleted: bool,
    ) -> Result<PageRow, AppError> {
        let sql = if include_deleted {
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ?")
        } else {
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL")
        };
        sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))
    }

    async fn ensure_page_handle_available(
        &self,
        handle: &str,
        exclude: Option<&PageId>,
    ) -> Result<(), AppError> {
        let exists: i64 = match exclude {
            Some(id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(id.as_str())
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if exists > 0 {
            return Err(AppError::conflict(format!(
                "page handle already exists: {handle}"
            )));
        }
        Ok(())
    }

    async fn fetch_navigation_row(&self, id: &NavigationId) -> Result<NavigationRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE id = ?");
        sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", id))
    }

    async fn fetch_navigation_row_by_handle(
        &self,
        handle: &str,
    ) -> Result<NavigationRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE handle = ?");
        sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(handle)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", handle))
    }

    async fn ensure_navigation_handle_available(
        &self,
        handle: &str,
        exclude: Option<&NavigationId>,
    ) -> Result<(), AppError> {
        let exists: i64 = match exclude {
            Some(id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM navigations WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(id.as_str())
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM navigations WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if exists > 0 {
            return Err(AppError::conflict(format!(
                "navigation handle already exists: {handle}"
            )));
        }
        Ok(())
    }

    async fn items_for_navigation(
        &self,
        navigation_id: &NavigationId,
    ) -> Result<Vec<NavigationItem>, AppError> {
        let sql = format!(
            "SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE navigation_id = ? \
             ORDER BY rank ASC, id ASC"
        );
        let rows = sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(navigation_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows.into_iter().map(NavigationItemRow::into_item).collect())
    }

    async fn fetch_navigation_item_row(
        &self,
        item_id: &str,
    ) -> Result<NavigationItemRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE id = ?");
        sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(item_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation item", item_id))
    }

    async fn fetch_navigation_item(&self, item_id: &str) -> Result<NavigationItem, AppError> {
        self.fetch_navigation_item_row(item_id)
            .await
            .map(NavigationItemRow::into_item)
    }

    fn validated_item_title(&self, title: &str) -> Result<String, AppError> {
        validate_length(title, "title", MAX_TITLE_LENGTH)?;
        Ok(title.trim().to_owned())
    }

    fn validated_item_url(&self, url: &str) -> Result<String, AppError> {
        validate_length(url, "url", MAX_URL_LENGTH)?;
        Ok(url.trim().to_owned())
    }

    async fn ensure_parent_in_navigation(
        &self,
        navigation_id: &NavigationId,
        parent_id: &str,
        item_id: Option<&str>,
    ) -> Result<(), AppError> {
        if item_id == Some(parent_id) {
            return Err(AppError::validation(
                "navigation item cannot be its own parent",
            ));
        }

        let mut current = Some(parent_id.to_owned());
        let mut steps = 0_usize;
        while let Some(current_id) = current {
            if Some(current_id.as_str()) == item_id {
                return Err(AppError::validation(
                    "navigation item parent would create a cycle",
                ));
            }
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT navigation_id, parent_id FROM navigation_items WHERE id = ?",
            )
            .bind(&current_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
            let Some((owner, next_parent)) = row else {
                return Err(AppError::validation(
                    "navigation item parent does not exist",
                ));
            };
            if owner != navigation_id.as_str() {
                return Err(AppError::validation(
                    "navigation item parent must belong to the same navigation",
                ));
            }
            current = next_parent;
            steps += 1;
            if steps > 10_000 {
                return Err(AppError::validation(
                    "navigation item hierarchy is too deep",
                ));
            }
        }
        Ok(())
    }
}

fn validate_rank(rank: i64) -> Result<(), AppError> {
    if rank < 0 {
        return Err(AppError::validation("rank must not be negative"));
    }
    Ok(())
}

fn storefront_error(error: StorefrontError) -> AppError {
    AppError::validation(error.to_string())
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(message)
        }
        _ => database_error(error),
    }
}

fn revision_conflict(current: i64) -> AppError {
    AppError::conflict_details(
        format!("page revision conflict: current revision is {current}"),
        serde_json::json!({ "current_revision": current }),
    )
}

/// Builds a case-insensitive `LIKE` pattern, escaping wildcards.
fn like_pattern(query: Option<&str>) -> Option<String> {
    let query = query?.trim();
    if query.is_empty() {
        return None;
    }
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for character in query.chars() {
        if matches!(character, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern.push('%');
    Some(pattern)
}

async fn next_page_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: &PageId,
) -> Result<i64, AppError> {
    sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM page_versions WHERE page_id = ?")
        .bind(page_id.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)
}

#[allow(clippy::too_many_arguments)]
async fn insert_page_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    page_id: &PageId,
    version: i64,
    body: &str,
    created_by: Option<&str>,
    kind: PageVersionKind,
    origin: PageRevisionOrigin,
    created_at: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO page_versions \
         (id, page_id, version, body, created_by, kind, origin, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(generate_id("pver"))
    .bind(page_id.as_str())
    .bind(version)
    .bind(body)
    .bind(created_by)
    .bind(kind.as_str())
    .bind(origin.as_str())
    .bind(created_at)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn serialize_json(value: &Value, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("could not serialize {field}: {error}")))
}

fn parse_json(value: &str) -> Value {
    match serde_json::from_str(value) {
        Ok(value) => value,
        Err(_) => Value::Null,
    }
}

fn navigation_tree_from(navigation: Navigation) -> NavigationTree {
    NavigationTree {
        id: navigation.id,
        name: navigation.name,
        handle: navigation.handle,
        metadata: navigation.metadata,
        items: build_navigation_tree(navigation.items),
    }
}

fn build_navigation_tree(items: Vec<NavigationItem>) -> Vec<NavigationTreeItem> {
    let mut children: HashMap<Option<String>, Vec<NavigationItem>> = HashMap::new();
    for item in items {
        children
            .entry(item.parent_id.clone())
            .or_default()
            .push(item);
    }
    let mut visited = HashSet::new();
    take_tree_children(&mut children, &None, &mut visited)
}

fn take_tree_children(
    children: &mut HashMap<Option<String>, Vec<NavigationItem>>,
    parent: &Option<String>,
    visited: &mut HashSet<String>,
) -> Vec<NavigationTreeItem> {
    let Some(mut items) = children.remove(parent) else {
        return Vec::new();
    };
    items.sort_by(|left, right| {
        left.rank
            .cmp(&right.rank)
            .then_with(|| left.id.cmp(&right.id))
    });
    items
        .into_iter()
        .filter_map(|item| {
            if !visited.insert(item.id.clone()) {
                return None;
            }
            let nested = take_tree_children(children, &Some(item.id.clone()), visited);
            Some(NavigationTreeItem {
                id: item.id,
                navigation_id: item.navigation_id,
                parent_id: item.parent_id,
                title: item.title,
                url: item.url,
                rank: item.rank,
                metadata: item.metadata,
                children: nested,
            })
        })
        .collect()
}

#[derive(Debug, FromRow)]
struct PageRow {
    id: String,
    title: String,
    handle: String,
    status: String,
    page_type: String,
    body: String,
    published_body: Option<String>,
    published_title: Option<String>,
    published_handle: Option<String>,
    published_page_type: Option<String>,
    published_seo: Option<String>,
    published_metadata: Option<String>,
    seo: String,
    metadata: String,
    revision: i64,
    published_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    deleted_at: Option<i64>,
}

impl PageRow {
    fn into_page(self) -> Page {
        let status = match PageStatus::parse(&self.status) {
            Some(status) => status,
            None => PageStatus::Draft,
        };
        Page {
            id: PageId::from_string(self.id),
            title: self.title,
            handle: self.handle,
            status,
            page_type: self.page_type,
            revision: self.revision,
            body: parse_json(&self.body),
            published_body: self.published_body.as_deref().map(parse_json),
            seo: parse_json(&self.seo),
            metadata: Metadata::from_json_str(&self.metadata),
            published_at: self.published_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    /// Projects the published snapshot, falling back per field to the live
    /// column for legacy rows that predate a snapshot column.
    fn into_published_page(self) -> Page {
        let status = match PageStatus::parse(&self.status) {
            Some(status) => status,
            None => PageStatus::Draft,
        };
        let body = self
            .published_body
            .as_deref()
            .map_or_else(|| parse_json(&self.body), parse_json);
        let seo = self
            .published_seo
            .as_deref()
            .map_or_else(|| parse_json(&self.seo), parse_json);
        let metadata = self.published_metadata.as_deref().map_or_else(
            || Metadata::from_json_str(&self.metadata),
            Metadata::from_json_str,
        );
        Page {
            id: PageId::from_string(self.id),
            title: self.published_title.unwrap_or(self.title),
            handle: self.published_handle.unwrap_or(self.handle),
            status,
            page_type: self.published_page_type.unwrap_or(self.page_type),
            revision: self.revision,
            body,
            published_body: None,
            seo,
            metadata,
            published_at: self.published_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    /// Returns true when every published snapshot field equals its live value.
    fn published_matches_live(&self) -> bool {
        self.published_title.as_deref() == Some(self.title.as_str())
            && self.published_handle.as_deref() == Some(self.handle.as_str())
            && self.published_page_type.as_deref() == Some(self.page_type.as_str())
            && self
                .published_body
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.body))
            && self
                .published_seo
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.seo))
            && self
                .published_metadata
                .as_deref()
                .is_some_and(|published| parse_json(published) == parse_json(&self.metadata))
    }
}

#[derive(Debug, FromRow)]
struct PageVersionRow {
    id: String,
    page_id: String,
    version: i64,
    body: String,
    created_by: Option<String>,
    kind: String,
    origin: String,
    created_at: i64,
}

impl PageVersionRow {
    fn into_version(self) -> PageVersion {
        PageVersion {
            id: self.id,
            page_id: PageId::from_string(self.page_id),
            version: self.version,
            body: parse_json(&self.body),
            kind: PageVersionKind::parse(&self.kind).unwrap_or(PageVersionKind::Published),
            origin: PageRevisionOrigin::parse(&self.origin).unwrap_or(PageRevisionOrigin::Manual),
            created_by: self.created_by,
            created_at: self.created_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct NavigationRow {
    id: String,
    name: String,
    handle: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl NavigationRow {
    fn into_navigation(self, items: Vec<NavigationItem>) -> Navigation {
        Navigation {
            id: NavigationId::from_string(self.id),
            name: self.name,
            handle: self.handle,
            metadata: Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            items,
        }
    }
}

#[derive(Debug, FromRow)]
struct NavigationItemRow {
    id: String,
    navigation_id: String,
    parent_id: Option<String>,
    title: String,
    url: String,
    rank: i64,
    metadata: String,
}

impl NavigationItemRow {
    fn into_item(self) -> NavigationItem {
        NavigationItem {
            id: self.id,
            navigation_id: NavigationId::from_string(self.navigation_id),
            parent_id: self.parent_id,
            title: self.title,
            url: self.url,
            rank: self.rank,
            metadata: Metadata::from_json_str(&self.metadata),
        }
    }
}

#[derive(Debug, FromRow)]
struct RedirectRow {
    id: String,
    from_path: String,
    to_path: String,
    status_code: i64,
    created_at: i64,
}

impl RedirectRow {
    fn into_redirect(self) -> Redirect {
        Redirect {
            id: self.id,
            from_path: self.from_path,
            to_path: self.to_path,
            status_code: self.status_code,
            created_at: self.created_at,
        }
    }
}
