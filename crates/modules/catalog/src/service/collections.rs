//! Collection authoring and published routes.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CollectionId, PageResult, Pagination, now_timestamp, validate_handle,
    validate_metadata,
};

use crate::domain::{
    Collection, CreateCollectionInput, PublishedRouteEntry, UpdateCollectionInput,
};

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;
impl CatalogService {
    /// Creates a collection.
    pub async fn create_collection(
        &self,
        input: CreateCollectionInput,
    ) -> Result<Collection, AppError> {
        let title = validate_title(&input.title, "collection title")?;
        let handle = resolve_handle(input.handle.as_deref(), &title, "collection handle")?;
        validate_metadata(&input.metadata)?;
        self.ensure_collection_handle_available(&handle, None)
            .await?;

        let id = CollectionId::new().as_str().to_owned();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO collections \
             (id, title, handle, description, is_published, rank, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&title)
        .bind(&handle)
        .bind(normalize_optional(input.description))
        .bind(bool_to_int(input.is_published.unwrap_or(false)))
        .bind(input.rank.unwrap_or(0))
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "collection handle already exists"))?;
        self.get_collection(&id).await
    }

    /// Loads a non-deleted collection.
    /// Soft-deletes a collection so historical references remain valid.
    pub async fn delete_collection(&self, id: &str) -> Result<(), AppError> {
        let now = fastcart_kernel::now_timestamp();
        let result = sqlx::query(
            "UPDATE collections SET deleted_at = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("collection", id));
        }
        Ok(())
    }

    pub async fn get_collection(&self, id: &str) -> Result<Collection, AppError> {
        let row = sqlx::query("SELECT * FROM collections WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("collection", id))?;
        collection_from_row(&row)
    }

    /// Loads a published, non-deleted collection by handle for storefronts.
    pub async fn get_published_collection_by_handle(
        &self,
        handle: &str,
    ) -> Result<Collection, AppError> {
        let row = sqlx::query(
            "SELECT * FROM collections \
             WHERE handle = ? AND deleted_at IS NULL AND is_published = 1",
        )
        .bind(handle)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("published collection handle", handle))?;
        collection_from_row(&row)
    }

    /// Partially updates a collection.
    pub async fn update_collection(
        &self,
        id: &str,
        input: UpdateCollectionInput,
    ) -> Result<Collection, AppError> {
        let existing = self.get_collection(id).await?;
        let title = match input.title.as_deref() {
            Some(value) => validate_title(value, "collection title")?,
            None => existing.title.clone(),
        };
        let handle = match input.handle.as_deref() {
            Some(value) => {
                let handle = value.trim().to_owned();
                validate_handle(&handle, "collection handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        if handle != existing.handle {
            self.ensure_collection_handle_available(&handle, Some(id))
                .await?;
        }
        let description = match input.description {
            Some(value) => normalize_optional(Some(value)),
            None => existing.description.clone(),
        };
        let metadata = match input.metadata {
            Some(metadata) => {
                validate_metadata(&metadata)?;
                metadata
            }
            None => existing.metadata.clone(),
        };

        sqlx::query(
            "UPDATE collections SET title = ?, handle = ?, description = ?, is_published = ?, \
             rank = ?, metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&title)
        .bind(&handle)
        .bind(&description)
        .bind(bool_to_int(
            input.is_published.unwrap_or(existing.is_published),
        ))
        .bind(input.rank.unwrap_or(existing.rank))
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "collection handle already exists"))?;
        self.get_collection(id).await
    }

    /// Lists non-deleted collections.
    pub async fn list_collections(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Collection>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM collections WHERE deleted_at IS NULL")
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT * FROM collections WHERE deleted_at IS NULL \
             ORDER BY rank ASC, created_at ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut collections = Vec::with_capacity(rows.len());
        for row in &rows {
            collections.push(collection_from_row(row)?);
        }
        Ok(PageResult::new(collections, count, pagination))
    }

    /// Searches non-deleted collections by title or handle.
    ///
    /// The match is a case-insensitive substring search. LIKE wildcards in the
    /// caller's query are escaped so a literal `%` or `_` is not treated as a
    /// pattern. An absent or blank query returns all non-deleted collections.
    pub async fn search_collections(
        &self,
        query: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Collection>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let pattern = query
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(|query| format!("%{}%", escape_like(query)));
        let condition = match pattern {
            Some(_) => {
                "deleted_at IS NULL AND (LOWER(title) LIKE LOWER(?) ESCAPE '\\' \
                 OR LOWER(handle) LIKE LOWER(?) ESCAPE '\\')"
            }
            None => "deleted_at IS NULL",
        };

        let count_sql = format!("SELECT COUNT(*) FROM collections WHERE {condition}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT * FROM collections WHERE {condition} \
             ORDER BY rank ASC, created_at ASC, id ASC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut collections = Vec::with_capacity(rows.len());
        for row in &rows {
            collections.push(collection_from_row(row)?);
        }
        Ok(PageResult::new(collections, count, pagination))
    }

    /// Lists published collections for storefronts.
    pub async fn list_published_collections(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Collection>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM collections WHERE deleted_at IS NULL AND is_published = 1",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT * FROM collections WHERE deleted_at IS NULL AND is_published = 1              ORDER BY rank ASC, created_at ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut collections = Vec::with_capacity(rows.len());
        for row in &rows {
            collections.push(collection_from_row(row)?);
        }
        Ok(PageResult::new(collections, count, pagination))
    }

    /// Lists handles of active, non-deleted products for sitemaps.
    pub async fn list_published_product_handles(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<String>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM products WHERE deleted_at IS NULL AND status = 'active'",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        let handles = sqlx::query_scalar(
            "SELECT handle FROM products WHERE deleted_at IS NULL AND status = 'active' \
             ORDER BY handle ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(PageResult::new(handles, count, pagination))
    }

    /// Lists handles of published, non-deleted collections for sitemaps.
    /// Lists published products with their sitemap timestamp.
    pub async fn list_published_product_entries(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<PublishedRouteEntry>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM products WHERE deleted_at IS NULL AND status = 'active'",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT handle, strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch') \
             FROM products WHERE deleted_at IS NULL AND status = 'active' \
             ORDER BY handle ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(PageResult::new(
            rows.into_iter()
                .map(|(handle, lastmod)| PublishedRouteEntry { handle, lastmod })
                .collect(),
            count,
            pagination,
        ))
    }

    /// Lists published collections with their sitemap timestamp.
    pub async fn list_published_collection_entries(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<PublishedRouteEntry>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM collections WHERE deleted_at IS NULL AND is_published = 1",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT handle, strftime('%Y-%m-%dT%H:%M:%SZ', updated_at, 'unixepoch') \
             FROM collections WHERE deleted_at IS NULL AND is_published = 1 \
             ORDER BY handle ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(PageResult::new(
            rows.into_iter()
                .map(|(handle, lastmod)| PublishedRouteEntry { handle, lastmod })
                .collect(),
            count,
            pagination,
        ))
    }

    pub async fn list_published_collection_handles(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<String>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM collections WHERE deleted_at IS NULL AND is_published = 1",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        let handles = sqlx::query_scalar(
            "SELECT handle FROM collections WHERE deleted_at IS NULL AND is_published = 1 \
             ORDER BY handle ASC, id ASC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(PageResult::new(handles, count, pagination))
    }

    /// Publishes a collection.
    pub async fn publish_collection(&self, id: &str) -> Result<Collection, AppError> {
        self.set_collection_published(id, true).await
    }

    /// Unpublishes a collection.
    pub async fn unpublish_collection(&self, id: &str) -> Result<Collection, AppError> {
        self.set_collection_published(id, false).await
    }

    /// Links a product to a collection, replacing the link rank when present.
    pub async fn add_product_to_collection(
        &self,
        product_id: &str,
        collection_id: &str,
        rank: i64,
    ) -> Result<(), AppError> {
        self.ensure_product_exists(product_id).await?;
        self.get_collection(collection_id).await?;
        sqlx::query(
            "INSERT INTO product_collection_links (product_id, collection_id, rank) \
             VALUES (?, ?, ?) \
             ON CONFLICT(product_id, collection_id) DO UPDATE SET rank = excluded.rank",
        )
        .bind(product_id)
        .bind(collection_id)
        .bind(rank)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Removes a product from a collection.
    pub async fn remove_product_from_collection(
        &self,
        product_id: &str,
        collection_id: &str,
    ) -> Result<(), AppError> {
        let result = sqlx::query(
            "DELETE FROM product_collection_links WHERE product_id = ? AND collection_id = ?",
        )
        .bind(product_id)
        .bind(collection_id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found(
                "product collection link",
                collection_id,
            ));
        }
        Ok(())
    }

    async fn set_collection_published(
        &self,
        id: &str,
        published: bool,
    ) -> Result<Collection, AppError> {
        let result = sqlx::query(
            "UPDATE collections SET is_published = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(bool_to_int(published))
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("collection", id));
        }
        self.get_collection(id).await
    }

    async fn ensure_collection_handle_available(
        &self,
        handle: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), AppError> {
        let count: i64 = match exclude_id {
            Some(exclude_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM collections WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(exclude_id)
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM collections WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if count > 0 {
            return Err(AppError::conflict(format!(
                "collection handle already exists: {handle}"
            )));
        }
        Ok(())
    }
}
