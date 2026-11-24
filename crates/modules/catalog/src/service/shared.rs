//! Shared converters, validation and cross-feature transactions.

use std::collections::HashMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, Metadata, Pagination, slugify, validate_handle, validate_length};
use sqlx::Row;
use sqlx::Transaction;
use sqlx::sqlite::{Sqlite, SqliteRow};

use crate::domain::{
    Category, Collection, Dimensions, Product, ProductImage, ProductOption, ProductOptionValue,
    ProductStatus, ProductVariant, Tag,
};

use super::*;

pub(super) fn column<'r, T>(row: &'r SqliteRow, name: &str) -> Result<T, AppError>
where
    T: sqlx::Decode<'r, Sqlite> + sqlx::Type<Sqlite>,
{
    row.try_get(name).map_err(database_error)
}

pub(super) fn metadata_column(row: &SqliteRow, name: &str) -> Result<Metadata, AppError> {
    let value: String = column(row, name)?;
    Ok(Metadata::from_json_str(&value))
}

pub(super) fn product_from_row(row: &SqliteRow) -> Result<Product, AppError> {
    let status: String = column(row, "status")?;
    Ok(Product {
        id: column(row, "id")?,
        title: column(row, "title")?,
        subtitle: column(row, "subtitle")?,
        description: column(row, "description")?,
        handle: column(row, "handle")?,
        status: ProductStatus::from_db(&status),
        thumbnail: column(row, "thumbnail")?,
        weight: column(row, "weight")?,
        dimensions: Dimensions {
            length: column(row, "length")?,
            height: column(row, "height")?,
            width: column(row, "width")?,
        },
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
        variants: Vec::new(),
        options: Vec::new(),
        images: Vec::new(),
        categories: Vec::new(),
        collections: Vec::new(),
        tags: Vec::new(),
        sales_channel_ids: Vec::new(),
    })
}

pub(super) fn variant_from_row(row: &SqliteRow) -> Result<ProductVariant, AppError> {
    let status: String = column(row, "status")?;
    let manage_inventory: i64 = column(row, "manage_inventory")?;
    let allow_backorder: i64 = column(row, "allow_backorder")?;
    Ok(ProductVariant {
        id: column(row, "id")?,
        product_id: column(row, "product_id")?,
        title: column(row, "title")?,
        sku: column(row, "sku")?,
        barcode: column(row, "barcode")?,
        status: ProductStatus::from_db(&status),
        weight: column(row, "weight")?,
        dimensions: Dimensions {
            length: column(row, "length")?,
            height: column(row, "height")?,
            width: column(row, "width")?,
        },
        manage_inventory: manage_inventory != 0,
        allow_backorder: allow_backorder != 0,
        tax_category_id: column(row, "tax_category_id")?,
        metadata: metadata_column(row, "metadata")?,
        option_value_ids: Vec::new(),
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
        deleted_at: column(row, "deleted_at")?,
    })
}

pub(super) fn option_from_row(row: &SqliteRow) -> Result<ProductOption, AppError> {
    Ok(ProductOption {
        id: column(row, "id")?,
        product_id: column(row, "product_id")?,
        title: column(row, "title")?,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
        values: Vec::new(),
    })
}

pub(super) fn option_value_from_row(row: &SqliteRow) -> Result<ProductOptionValue, AppError> {
    Ok(ProductOptionValue {
        id: column(row, "id")?,
        option_id: column(row, "product_option_id")?,
        value: column(row, "value")?,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
    })
}

pub(super) fn image_from_row(row: &SqliteRow) -> Result<ProductImage, AppError> {
    Ok(ProductImage {
        id: column(row, "id")?,
        product_id: column(row, "product_id")?,
        variant_id: column(row, "variant_id")?,
        url: column(row, "url")?,
        alt: column(row, "alt")?,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
    })
}

pub(super) fn category_from_row(row: &SqliteRow) -> Result<Category, AppError> {
    let is_active: i64 = column(row, "is_active")?;
    let is_internal: i64 = column(row, "is_internal")?;
    Ok(Category {
        id: column(row, "id")?,
        parent_id: column(row, "parent_id")?,
        title: column(row, "title")?,
        handle: column(row, "handle")?,
        description: column(row, "description")?,
        is_active: is_active != 0,
        is_internal: is_internal != 0,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

pub(super) fn collection_from_row(row: &SqliteRow) -> Result<Collection, AppError> {
    let is_published: i64 = column(row, "is_published")?;
    Ok(Collection {
        id: column(row, "id")?,
        title: column(row, "title")?,
        handle: column(row, "handle")?,
        description: column(row, "description")?,
        is_published: is_published != 0,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

pub(super) fn tag_from_row(row: &SqliteRow) -> Result<Tag, AppError> {
    Ok(Tag {
        id: column(row, "id")?,
        value: column(row, "value")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
    })
}

pub(super) fn validate_title(value: &str, field: &str) -> Result<String, AppError> {
    validate_length(value, field, MAX_TITLE_LENGTH)?;
    Ok(value.trim().to_owned())
}

pub(super) fn resolve_handle(
    value: Option<&str>,
    title: &str,
    field: &str,
) -> Result<String, AppError> {
    let handle = match value {
        Some(value) => value.trim().to_owned(),
        None => slugify(title),
    };
    validate_handle(&handle, field)?;
    Ok(handle)
}

pub(super) fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn normalize_key(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(super) fn validate_weight(weight: Option<f64>) -> Result<(), AppError> {
    if let Some(weight) = weight
        && (!weight.is_finite() || weight < 0.0)
    {
        return Err(AppError::validation(
            "weight must be a finite, non-negative number",
        ));
    }
    Ok(())
}

pub(super) fn validate_dimensions(dimensions: Dimensions) -> Result<(), AppError> {
    for (value, field) in [
        (dimensions.length, "length"),
        (dimensions.height, "height"),
        (dimensions.width, "width"),
    ] {
        if let Some(value) = value
            && (!value.is_finite() || value < 0.0)
        {
            return Err(AppError::validation(format!(
                "{field} must be a finite, non-negative number"
            )));
        }
    }
    Ok(())
}

pub(super) fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

pub(super) fn checked_pagination(pagination: Pagination) -> Result<Pagination, AppError> {
    Pagination::new(Some(pagination.limit), Some(pagination.offset))
}

pub(super) fn sql_markers(count: usize) -> String {
    vec!["?"; count].join(", ")
}

/// Escapes LIKE metacharacters so user input matches literally.
pub(super) fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

pub(super) fn option_map_from_rows(
    rows: &[SqliteRow],
) -> Result<HashMap<(String, String), String>, AppError> {
    let mut map = HashMap::new();
    for row in rows {
        let option_title: String = column(row, "option_title")?;
        let value: String = column(row, "value")?;
        let value_id: String = column(row, "value_id")?;
        map.insert(
            (normalize_key(&option_title), normalize_key(&value)),
            value_id,
        );
    }
    Ok(map)
}

pub(super) async fn ensure_product_exists_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<(), AppError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?;
    if count == 0 {
        return Err(AppError::not_found("product", id));
    }
    Ok(())
}

pub(super) async fn ensure_variant_belongs_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variant_id: &str,
) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM product_variants \
         WHERE id = ? AND product_id = ? AND deleted_at IS NULL",
    )
    .bind(variant_id)
    .bind(product_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    if count == 0 {
        return Err(AppError::not_found("variant", variant_id));
    }
    Ok(())
}

pub(super) async fn sync_search_index_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_search WHERE product_id = ?")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    let row =
        sqlx::query("SELECT title, description, handle, deleted_at FROM products WHERE id = ?")
            .bind(product_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    let Some(row) = row else {
        return Ok(());
    };
    let deleted_at: Option<i64> = column(&row, "deleted_at")?;
    if deleted_at.is_some() {
        return Ok(());
    }
    let title: String = column(&row, "title")?;
    let description: Option<String> = column(&row, "description")?;
    let handle: String = column(&row, "handle")?;
    let skus: Vec<String> = sqlx::query_scalar(
        "SELECT sku FROM product_variants \
         WHERE product_id = ? AND sku IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;

    sqlx::query(
        "INSERT INTO product_search (product_id, title, description, sku, handle) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(product_id)
    .bind(title)
    .bind(description.unwrap_or_default())
    .bind(skus.join(" "))
    .bind(handle)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}
