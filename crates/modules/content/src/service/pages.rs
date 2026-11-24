//! Page authoring, lookup and storefront reads.

use super::*;

impl ContentService {
    /// Validates every non-deleted page body against a candidate theme.
    ///
    /// Empty bodies are skipped because they carry no theme-bound content.
    /// Pages are read in batches of [`Pagination::MAX_LIMIT`] so a large store
    /// is never loaded at once. Each returned issue identifies the page and the
    /// first validation failure, which lets theme activation report every page
    /// that cannot render under the candidate theme.
    ///
    /// A page authored under a different theme is still validated: activation
    /// is the migration point between themes, so the question is whether the
    /// blocks survive the new editor contract, not which theme stamped them.
    pub async fn validate_all_pages(
        &self,
        editor_manifest: &ThemeEditorManifest,
    ) -> Result<Vec<PageValidationIssue>, AppError> {
        let mut issues = Vec::new();
        let mut offset = 0_i64;
        loop {
            let pagination = Pagination::new(Some(Pagination::MAX_LIMIT), Some(offset))?;
            let page = self.list_pages(None, false, pagination).await?;
            let batch_len = page.data.len();
            for record in page.data {
                // Archived pages are withdrawn from every storefront surface,
                // so they must not block theme activation.
                if record.status == PageStatus::Archived || is_empty_body(&record.body) {
                    continue;
                }
                if let Err(error) = validate_page_body_for_theme(&record.body, editor_manifest) {
                    issues.push(PageValidationIssue {
                        page_id: record.id,
                        handle: record.handle,
                        error,
                    });
                }
            }
            offset = offset.saturating_add(pagination.limit);
            if batch_len < usize::try_from(pagination.limit).unwrap_or(usize::MAX) {
                break;
            }
        }
        Ok(issues)
    }
}

impl ContentService {
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
        let body = self.stamp_theme_ref(body);
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
}

impl ContentService {
    /// Loads a page by identifier. Soft-deleted pages are not returned.
    pub async fn get_page(&self, id: &PageId) -> Result<Page, AppError> {
        self.fetch_page_row(id, false).await.map(PageRow::into_page)
    }
}

impl ContentService {
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
}

impl ContentService {
    /// Loads a published page by handle for storefront rendering.
    ///
    /// Only live, published pages are returned. Routing uses the published
    /// handle, falling back to the live handle only for legacy rows whose
    /// published snapshot predates the column. The returned [`Page`] carries
    /// the published title, handle, page type, body, SEO and metadata, falling
    /// back per field to the draft value when that snapshot value is absent,
    /// and always clears [`Page::published_body`] so the draft can never leak to
    /// storefronts.
    /// Ensures the default theme's system pages exist and are published.
    ///
    /// The cart and checkout funnels are client-side drawers, but deep links
    /// still need a routable page so `/cart` and `/checkout` render instead of
    /// returning 404. Existing pages (including drafts) are left untouched.
    pub async fn ensure_system_pages(&self) -> Result<(), AppError> {
        for (handle, title) in [("cart", "Cart"), ("checkout", "Checkout")] {
            match self.get_page_by_handle(handle).await {
                Ok(_) => continue,
                Err(AppError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
            let body = serde_json::json!({
                "schema_version": 1,
                "root": { "props": {} },
                "content": [{
                    "type": "Heading",
                    "props": { "id": format!("{handle}-title"), "level": "1", "text": title }
                }],
                "zones": {}
            });
            let page = self
                .create_page(CreatePageInput {
                    title: title.to_owned(),
                    handle: Some(handle.to_owned()),
                    page_type: Some("custom".to_owned()),
                    body,
                    seo: serde_json::json!({ "noindex": true }),
                    metadata: fastcart_kernel::Metadata::empty(),
                })
                .await?;
            self.publish_page(&page.id).await?;
        }
        Ok(())
    }
}

impl ContentService {
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
}

impl ContentService {
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
}

impl ContentService {
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
}

impl ContentService {
    /// Loads the newest published page used as a dynamic route template.
    ///
    /// Route templates are matched by published `page_type`, so a draft-only
    /// classifier change cannot start serving a different template.
    pub async fn get_published_page_by_type(&self, page_type: &str) -> Result<Page, AppError> {
        let sql = format!(
            "SELECT {PAGE_COLUMNS} FROM pages \
             WHERE status = 'published' AND deleted_at IS NULL \
               AND COALESCE(published_page_type, page_type) = ? \
             ORDER BY published_at DESC, id ASC LIMIT 1"
        );
        sqlx::query_as::<_, PageRow>(&sql)
            .bind(page_type)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page type", page_type))
            .map(PageRow::into_published_page)
    }
}

impl ContentService {
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
}

impl ContentService {
    /// Lists storefront-visible routes without loading page bodies.
    ///
    /// Only the published snapshot is projected, so drafts cannot leak handles
    /// or classifiers into sitemaps. Selecting three columns keeps the query
    /// cheap even for stores with many pages.
    pub async fn list_published_routes(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<PageRoute>, AppError> {
        const CONDITIONS: &str = " WHERE status = 'published' AND deleted_at IS NULL";
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM pages{CONDITIONS}"))
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let sql = format!(
            "SELECT id, COALESCE(published_handle, handle) AS handle, \
             COALESCE(published_page_type, page_type) AS page_type, \
             strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch') AS lastmod \
             FROM pages{CONDITIONS} ORDER BY handle ASC, id ASC LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, (String, String, String, String)>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let data = rows
            .into_iter()
            .map(|(id, handle, page_type, lastmod)| PageRoute {
                id: PageId::from_string(id),
                handle,
                page_type,
                lastmod,
            })
            .collect();
        Ok(PageResult::new(data, count, pagination))
    }
}

impl ContentService {
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
}

impl ContentService {
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
            Some(body) => self.stamp_theme_ref(body),
            // A body-less update leaves the stored body and its theme
            // reference untouched so unrelated edits stay possible.
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
}

impl ContentService {
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
}
