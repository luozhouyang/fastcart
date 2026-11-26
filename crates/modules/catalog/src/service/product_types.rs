//! Product type taxonomy.
//!
//! A product may reference at most one type. Types are store-managed values
//! with a unique display value; deleting a type detaches it from every product
//! in the same transaction, matching `ON DELETE SET NULL` semantics that SQLite
//! cannot attach to the pre-existing `products` table.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, PageResult, Pagination, ProductTypeId, now_timestamp, validate_length,
    validate_metadata,
};
use sqlx::Transaction;
use sqlx::sqlite::{Sqlite, SqliteRow};

use crate::domain::{CreateProductTypeInput, ProductType, UpdateProductTypeInput};

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;

/// Maximum product type value length.
pub const MAX_PRODUCT_TYPE_VALUE_LENGTH: usize = 100;

impl CatalogService {
    /// Creates a product type, rejecting duplicate values case-insensitively.
    pub async fn create_product_type(
        &self,
        input: CreateProductTypeInput,
    ) -> Result<ProductType, AppError> {
        let value = validate_product_type_value(&input.value)?;
        validate_metadata(&input.metadata)?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        ensure_product_type_value_available_tx(&mut tx, &value, None).await?;
        let id = ProductTypeId::new().as_str().to_owned();
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO product_types (id, value, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&value)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "product type value already exists"))?;
        tx.commit().await.map_err(database_error)?;
        self.get_product_type(&id).await
    }

    /// Loads a product type.
    pub async fn get_product_type(&self, id: &str) -> Result<ProductType, AppError> {
        let row = sqlx::query("SELECT * FROM product_types WHERE id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("product type", id))?;
        product_type_from_row(&row)
    }

    /// Lists product types ordered by value.
    pub async fn list_product_types(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<ProductType>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_types")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows =
            sqlx::query("SELECT * FROM product_types ORDER BY value ASC, id ASC LIMIT ? OFFSET ?")
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
        let mut types = Vec::with_capacity(rows.len());
        for row in &rows {
            types.push(product_type_from_row(row)?);
        }
        Ok(PageResult::new(types, count, pagination))
    }

    /// Partially updates a product type.
    pub async fn update_product_type(
        &self,
        id: &str,
        input: UpdateProductTypeInput,
    ) -> Result<ProductType, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let existing_row = sqlx::query("SELECT * FROM product_types WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("product type", id))?;
        let existing = product_type_from_row(&existing_row)?;
        let value = match input.value.as_deref() {
            Some(value) => validate_product_type_value(value)?,
            None => existing.value.clone(),
        };
        if value != existing.value {
            ensure_product_type_value_available_tx(&mut tx, &value, Some(id)).await?;
        }
        let metadata = match input.metadata {
            Some(metadata) => {
                validate_metadata(&metadata)?;
                metadata
            }
            None => existing.metadata.clone(),
        };
        let result = sqlx::query(
            "UPDATE product_types SET value = ?, metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&value)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "product type value already exists"))?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("product type", id));
        }
        tx.commit().await.map_err(database_error)?;
        self.get_product_type(id).await
    }

    /// Deletes a product type and clears it from every product that used it.
    pub async fn delete_product_type(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_types WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
        if exists == 0 {
            return Err(AppError::not_found("product type", id));
        }
        let now = now_timestamp();
        sqlx::query(
            "UPDATE products SET product_type_id = NULL, updated_at = ? WHERE product_type_id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query("DELETE FROM product_types WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

fn product_type_from_row(row: &SqliteRow) -> Result<ProductType, AppError> {
    Ok(ProductType {
        id: column(row, "id")?,
        value: column(row, "value")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

/// Trims and validates a product type value.
fn validate_product_type_value(value: &str) -> Result<String, AppError> {
    validate_length(value, "product type value", MAX_PRODUCT_TYPE_VALUE_LENGTH)?;
    Ok(value.trim().to_owned())
}

pub(super) async fn ensure_product_type_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_types WHERE id = ?")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
    if count == 0 {
        return Err(AppError::not_found("product type", id));
    }
    Ok(())
}

/// Rejects a value already used by another type, compared without case or
/// surrounding whitespace so "  Shirt " and "shirt" collide.
async fn ensure_product_type_value_available_tx(
    tx: &mut Transaction<'_, Sqlite>,
    value: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let normalized = normalize_key(value);
    let count: i64 = match exclude_id {
        Some(exclude_id) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM product_types WHERE LOWER(TRIM(value)) = ? AND id <> ?",
        )
        .bind(&normalized)
        .bind(exclude_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?,
        None => {
            sqlx::query_scalar("SELECT COUNT(*) FROM product_types WHERE LOWER(TRIM(value)) = ?")
                .bind(&normalized)
                .fetch_one(&mut **tx)
                .await
                .map_err(database_error)?
        }
    };
    if count > 0 {
        return Err(AppError::conflict(format!(
            "product type value already exists: {value}"
        )));
    }
    Ok(())
}
