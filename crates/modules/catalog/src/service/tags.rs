//! Product tags.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, Metadata, PageResult, Pagination, TagId, now_timestamp, validate_length,
};
use sqlx::Transaction;
use sqlx::sqlite::Sqlite;

use crate::domain::Tag;

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;
impl CatalogService {
    /// Creates a tag by value, or returns the existing tag.
    pub async fn create_tag(&self, value: &str) -> Result<Tag, AppError> {
        let value = value.trim();
        validate_length(value, "tag value", MAX_TAG_VALUE_LENGTH)?;
        if let Some(row) = sqlx::query("SELECT * FROM tags WHERE value = ?")
            .bind(value)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
        {
            return tag_from_row(&row);
        }
        let id = TagId::new().as_str().to_owned();
        let now = now_timestamp();
        sqlx::query("INSERT INTO tags (id, value, metadata, created_at) VALUES (?, ?, '{}', ?)")
            .bind(&id)
            .bind(value)
            .bind(now)
            .execute(self.db.pool())
            .await
            .map_err(|error| unique_or_database(error, "tag value already exists"))?;
        Ok(Tag {
            id,
            value: value.to_owned(),
            metadata: Metadata::empty(),
            created_at: now,
        })
    }

    /// Lists tags.
    pub async fn list_tags(&self, pagination: Pagination) -> Result<PageResult<Tag>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query("SELECT * FROM tags ORDER BY value ASC, id ASC LIMIT ? OFFSET ?")
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut tags = Vec::with_capacity(rows.len());
        for row in &rows {
            tags.push(tag_from_row(row)?);
        }
        Ok(PageResult::new(tags, count, pagination))
    }

    /// Returns a tag by value.
    pub async fn get_tag_by_value(&self, value: &str) -> Result<Tag, AppError> {
        let row = sqlx::query("SELECT * FROM tags WHERE value = ?")
            .bind(value.trim())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("tag", value))?;
        tag_from_row(&row)
    }

    /// Attaches a tag value to a product, creating the tag when needed.
    pub async fn attach_tag(&self, product_id: &str, value: &str) -> Result<Tag, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        ensure_product_exists_tx(&mut tx, product_id).await?;
        let tag = get_or_create_tag_tx(&mut tx, value).await?;
        sqlx::query("INSERT OR IGNORE INTO product_tag_links (product_id, tag_id) VALUES (?, ?)")
            .bind(product_id)
            .bind(&tag.id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(tag)
    }

    /// Detaches a tag from a product.
    pub async fn detach_tag(&self, product_id: &str, tag_id: &str) -> Result<(), AppError> {
        let result =
            sqlx::query("DELETE FROM product_tag_links WHERE product_id = ? AND tag_id = ?")
                .bind(product_id)
                .bind(tag_id)
                .execute(self.db.pool())
                .await
                .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("product tag link", tag_id));
        }
        Ok(())
    }
}

async fn get_or_create_tag_tx(
    tx: &mut Transaction<'_, Sqlite>,
    value: &str,
) -> Result<Tag, AppError> {
    let value = value.trim();
    validate_length(value, "tag value", MAX_TAG_VALUE_LENGTH)?;
    if let Some(row) = sqlx::query("SELECT * FROM tags WHERE value = ?")
        .bind(value)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?
    {
        return tag_from_row(&row);
    }
    let id = TagId::new().as_str().to_owned();
    let now = now_timestamp();
    sqlx::query("INSERT INTO tags (id, value, metadata, created_at) VALUES (?, ?, '{}', ?)")
        .bind(&id)
        .bind(value)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(|error| unique_or_database(error, "tag value already exists"))?;
    Ok(Tag {
        id,
        value: value.to_owned(),
        metadata: Metadata::empty(),
        created_at: now,
    })
}
