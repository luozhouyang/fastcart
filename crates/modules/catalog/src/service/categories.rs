//! Category authoring and hierarchy.

use std::collections::HashSet;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CategoryId, PageResult, Pagination, now_timestamp, validate_handle, validate_metadata,
};
use sqlx::Transaction;
use sqlx::sqlite::Sqlite;

use crate::domain::{Category, CreateCategoryInput, UpdateCategoryInput};

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;
impl CatalogService {
    /// Creates a category, optionally nested under a parent.
    pub async fn create_category(&self, input: CreateCategoryInput) -> Result<Category, AppError> {
        let title = validate_title(&input.title, "category title")?;
        let handle = resolve_handle(input.handle.as_deref(), &title, "category handle")?;
        validate_metadata(&input.metadata)?;
        self.ensure_category_handle_available(&handle, None).await?;
        let parent_id = normalize_optional(input.parent_id);
        if let Some(parent_id) = parent_id.as_deref() {
            self.ensure_category_exists(parent_id).await?;
        }

        let id = CategoryId::new().as_str().to_owned();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO categories \
             (id, parent_id, title, handle, description, is_active, is_internal, rank, metadata, \
              created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&parent_id)
        .bind(&title)
        .bind(&handle)
        .bind(normalize_optional(input.description))
        .bind(bool_to_int(input.is_active.unwrap_or(true)))
        .bind(bool_to_int(input.is_internal.unwrap_or(false)))
        .bind(input.rank.unwrap_or(0))
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "category handle already exists"))?;
        self.get_category(&id).await
    }

    /// Loads a non-deleted category.
    pub async fn get_category(&self, id: &str) -> Result<Category, AppError> {
        let row = sqlx::query("SELECT * FROM categories WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("category", id))?;
        category_from_row(&row)
    }

    /// Partially updates a category, rejecting hierarchy cycles.
    pub async fn update_category(
        &self,
        id: &str,
        input: UpdateCategoryInput,
    ) -> Result<Category, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let existing_row =
            sqlx::query("SELECT * FROM categories WHERE id = ? AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?
                .ok_or_else(|| AppError::not_found("category", id))?;
        let existing = category_from_row(&existing_row)?;
        let title = match input.title.as_deref() {
            Some(value) => validate_title(value, "category title")?,
            None => existing.title.clone(),
        };
        let handle = match input.handle.as_deref() {
            Some(value) => {
                let handle = value.trim().to_owned();
                validate_handle(&handle, "category handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        if handle != existing.handle {
            ensure_category_handle_available_tx(&mut tx, &handle, Some(id)).await?;
        }
        let parent_id = match input.parent_id {
            Some(value) => {
                let parent_id = normalize_optional(Some(value));
                if let Some(parent_id) = parent_id.as_deref() {
                    ensure_category_exists_tx(&mut tx, parent_id).await?;
                    ensure_no_category_cycle_tx(&mut tx, id, parent_id).await?;
                }
                parent_id
            }
            None => existing.parent_id.clone(),
        };
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

        let result = sqlx::query(
            "UPDATE categories SET parent_id = ?, title = ?, handle = ?, description = ?, \
             is_active = ?, is_internal = ?, rank = ?, metadata = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(&parent_id)
        .bind(&title)
        .bind(&handle)
        .bind(&description)
        .bind(bool_to_int(input.is_active.unwrap_or(existing.is_active)))
        .bind(bool_to_int(
            input.is_internal.unwrap_or(existing.is_internal),
        ))
        .bind(input.rank.unwrap_or(existing.rank))
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "category handle already exists"))?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("category", id));
        }
        tx.commit().await.map_err(database_error)?;
        self.get_category(id).await
    }

    /// Lists non-deleted categories. When `parent_id` is supplied only direct
    /// children are returned.
    pub async fn list_categories(
        &self,
        parent_id: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Category>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let (condition, bind) = match parent_id {
            Some(parent_id) => (
                "deleted_at IS NULL AND parent_id = ?",
                Some(parent_id.to_owned()),
            ),
            None => ("deleted_at IS NULL", None),
        };
        let count_sql = format!("SELECT COUNT(*) FROM categories WHERE {condition}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(bind) = bind.as_ref() {
            count_query = count_query.bind(bind);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT * FROM categories WHERE {condition} \
             ORDER BY rank ASC, title ASC, id ASC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query(&list_sql);
        if let Some(bind) = bind.as_ref() {
            list_query = list_query.bind(bind);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut categories = Vec::with_capacity(rows.len());
        for row in &rows {
            categories.push(category_from_row(row)?);
        }
        Ok(PageResult::new(categories, count, pagination))
    }

    /// Lists active, non-internal categories for storefronts.
    pub async fn list_published_categories(
        &self,
        parent_id: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Category>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let (condition, bind) = match parent_id {
            Some(parent_id) => (
                "deleted_at IS NULL AND is_active = 1 AND is_internal = 0 AND parent_id = ?",
                Some(parent_id.to_owned()),
            ),
            None => (
                "deleted_at IS NULL AND is_active = 1 AND is_internal = 0",
                None,
            ),
        };
        let count_sql = format!("SELECT COUNT(*) FROM categories WHERE {condition}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(bind) = bind.as_ref() {
            count_query = count_query.bind(bind);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let list_sql = format!(
            "SELECT * FROM categories WHERE {condition}              ORDER BY rank ASC, title ASC, id ASC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query(&list_sql);
        if let Some(bind) = bind.as_ref() {
            list_query = list_query.bind(bind);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut categories = Vec::with_capacity(rows.len());
        for row in &rows {
            categories.push(category_from_row(row)?);
        }
        Ok(PageResult::new(categories, count, pagination))
    }

    /// Soft-deletes a category after checking for children and linked products.
    pub async fn delete_category(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM categories WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        if exists == 0 {
            return Err(AppError::not_found("category", id));
        }
        let children: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM categories WHERE parent_id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        if children > 0 {
            return Err(AppError::conflict(
                "category has children and cannot be deleted",
            ));
        }
        let linked: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM product_category_links WHERE category_id = ?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if linked > 0 {
            return Err(AppError::conflict(
                "category has linked products and cannot be deleted",
            ));
        }
        let result = sqlx::query(
            "UPDATE categories SET deleted_at = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("category", id));
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    async fn ensure_category_exists(&self, id: &str) -> Result<(), AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM categories WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        if count == 0 {
            return Err(AppError::not_found("category", id));
        }
        Ok(())
    }

    async fn ensure_category_handle_available(
        &self,
        handle: &str,
        exclude_id: Option<&str>,
    ) -> Result<(), AppError> {
        let count: i64 = match exclude_id {
            Some(exclude_id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(exclude_id)
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if count > 0 {
            return Err(AppError::conflict(format!(
                "category handle already exists: {handle}"
            )));
        }
        Ok(())
    }
}

async fn ensure_category_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<(), AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?;
    if count == 0 {
        return Err(AppError::not_found("category", id));
    }
    Ok(())
}

async fn ensure_category_handle_available_tx(
    tx: &mut Transaction<'_, Sqlite>,
    handle: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let count: i64 = match exclude_id {
        Some(exclude_id) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE handle = ? AND id <> ?")
                .bind(handle)
                .bind(exclude_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(database_error)?
        }
        None => sqlx::query_scalar("SELECT COUNT(*) FROM categories WHERE handle = ?")
            .bind(handle)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?,
    };
    if count > 0 {
        return Err(AppError::conflict(format!(
            "category handle already exists: {handle}"
        )));
    }
    Ok(())
}

async fn ensure_no_category_cycle_tx(
    tx: &mut Transaction<'_, Sqlite>,
    category_id: &str,
    parent_id: &str,
) -> Result<(), AppError> {
    let mut current = Some(parent_id.to_owned());
    let mut visited = HashSet::new();
    while let Some(id) = current {
        if id == category_id {
            return Err(AppError::conflict("category cannot be its own ancestor"));
        }
        if !visited.insert(id.clone()) {
            return Err(AppError::conflict("category hierarchy contains a cycle"));
        }
        current = sqlx::query("SELECT parent_id FROM categories WHERE id = ?")
            .bind(&id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .map(|row| column::<Option<String>>(&row, "parent_id"))
            .transpose()?
            .flatten();
    }
    Ok(())
}
