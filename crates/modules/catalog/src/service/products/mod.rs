//! Product authoring, publishing and search.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, ImageId, OptionId, OptionValueId, PageResult, Pagination, ProductId, TagId,
    now_timestamp, validate_handle, validate_length, validate_metadata,
};
use sqlx::Transaction;
use sqlx::sqlite::Sqlite;

use crate::domain::{
    Category, Collection, CreateOptionInput, CreateProductInput, Product, ProductFilter,
    ProductImage, ProductOption, ProductOptionValue, ProductSort, ProductStatus, ProductVariant,
    Tag, UpdateProductInput,
};

use super::shared::*;
use super::variants::{insert_variants_tx, replace_variants_tx};
/// Full CRUD over the catalog aggregate.
use super::*;
fn product_filter_clause(filter: &ProductFilter) -> (String, Vec<String>) {
    let mut conditions = Vec::new();
    let mut binds = Vec::new();
    if !filter.include_deleted {
        conditions.push("p.deleted_at IS NULL".to_owned());
    }
    if let Some(status) = filter.status {
        conditions.push("p.status = ?".to_owned());
        binds.push(status.as_str().to_owned());
    }
    if let Some(category_id) = filter.category_id.as_ref() {
        conditions.push(
            "EXISTS (SELECT 1 FROM product_category_links l \
             WHERE l.product_id = p.id AND l.category_id = ?)"
                .to_owned(),
        );
        binds.push(category_id.clone());
    }
    if let Some(collection_id) = filter.collection_id.as_ref() {
        conditions.push(
            "EXISTS (SELECT 1 FROM product_collection_links l \
             WHERE l.product_id = p.id AND l.collection_id = ?)"
                .to_owned(),
        );
        binds.push(collection_id.clone());
    }
    if let Some(tag_value) = filter.tag_value.as_ref() {
        conditions.push(
            "EXISTS (SELECT 1 FROM product_tag_links l JOIN tags t ON t.id = l.tag_id \
             WHERE l.product_id = p.id AND t.value = ?)"
                .to_owned(),
        );
        binds.push(tag_value.clone());
    }
    if let Some(channel_ids) = filter
        .sales_channel_ids
        .as_ref()
        .filter(|channel_ids| !channel_ids.is_empty())
    {
        let markers = sql_markers(channel_ids.len());
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM product_sales_channels sc \
             WHERE sc.product_id = p.id AND sc.sales_channel_id IN ({markers}))"
        ));
        binds.extend(channel_ids.iter().cloned());
    }
    if let Some(query) = filter.query.as_ref() {
        let query = query.trim();
        if !query.is_empty() {
            let pattern = format!("%{query}%");
            conditions.push(
                "(p.title LIKE ? OR COALESCE(p.description, '') LIKE ? OR p.handle LIKE ? \
                 OR EXISTS (SELECT 1 FROM product_variants sv \
                            WHERE sv.product_id = p.id AND sv.deleted_at IS NULL \
                            AND COALESCE(sv.sku, '') LIKE ?))"
                    .to_owned(),
            );
            binds.push(pattern.clone());
            binds.push(pattern.clone());
            binds.push(pattern.clone());
            binds.push(pattern);
        }
    }
    if conditions.is_empty() {
        conditions.push("1 = 1".to_owned());
    }
    (conditions.join(" AND "), binds)
}

fn fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .flat_map(|term| term.split(|character: char| !character.is_alphanumeric()))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

mod lifecycle;
mod reads;
mod search;

async fn insert_options_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    options: &[CreateOptionInput],
) -> Result<HashMap<(String, String), String>, AppError> {
    let mut map = HashMap::new();
    let mut titles = HashSet::new();
    for (index, option) in options.iter().enumerate() {
        let title = validate_title(&option.title, "option title")?;
        if !titles.insert(normalize_key(&title)) {
            return Err(AppError::validation("duplicate option title"));
        }
        validate_metadata(&option.metadata)?;
        let option_id = OptionId::new().as_str().to_owned();
        let rank = match option.rank {
            Some(rank) => rank,
            None => i64::try_from(index).map_err(|_| AppError::validation("rank overflow"))?,
        };
        sqlx::query(
            "INSERT INTO product_options (id, product_id, title, rank, metadata) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&option_id)
        .bind(product_id)
        .bind(&title)
        .bind(rank)
        .bind(option.metadata.to_json_string())
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;

        let mut values = HashSet::new();
        for (value_index, value) in option.values.iter().enumerate() {
            let value = value.trim();
            validate_length(value, "option value", MAX_OPTION_VALUE_LENGTH)?;
            if !values.insert(normalize_key(value)) {
                return Err(AppError::validation("duplicate option value"));
            }
            let value_id = OptionValueId::new().as_str().to_owned();
            let rank =
                i64::try_from(value_index).map_err(|_| AppError::validation("rank overflow"))?;
            sqlx::query(
                "INSERT INTO product_option_values \
                 (id, product_option_id, value, rank, metadata) VALUES (?, ?, ?, ?, '{}')",
            )
            .bind(&value_id)
            .bind(&option_id)
            .bind(value)
            .bind(rank)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
            map.insert((normalize_key(&title), normalize_key(value)), value_id);
        }
    }
    Ok(map)
}

async fn option_map_from_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
) -> Result<HashMap<(String, String), String>, AppError> {
    let rows = sqlx::query(
        "SELECT o.title AS option_title, v.value AS value, v.id AS value_id \
         FROM product_option_values v \
         JOIN product_options o ON o.id = v.product_option_id \
         WHERE o.product_id = ?",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    option_map_from_rows(&rows)
}

/// Applies a full option replacement while preserving the identifiers of
/// options and values whose natural keys are unchanged.
///
/// This keeps `product_variant_option_values` links intact for variants that
/// are not resubmitted with the product update.
async fn sync_options_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    options: &[CreateOptionInput],
) -> Result<HashMap<(String, String), String>, AppError> {
    let existing_option_rows = sqlx::query_as::<_, (String, String)>(
        "SELECT id, title FROM product_options WHERE product_id = ?",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    let mut existing_options: HashMap<String, String> = existing_option_rows
        .into_iter()
        .map(|(id, title)| (normalize_key(&title), id))
        .collect();

    let existing_value_rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT v.id, v.product_option_id, v.value \
         FROM product_option_values v \
         JOIN product_options o ON o.id = v.product_option_id \
         WHERE o.product_id = ?",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;
    let mut existing_values: HashMap<(String, String), String> = existing_value_rows
        .into_iter()
        .map(|(id, option_id, value)| ((option_id, normalize_key(&value)), id))
        .collect();

    let mut map = HashMap::new();
    let mut titles = HashSet::new();
    for (index, option) in options.iter().enumerate() {
        let title = validate_title(&option.title, "option title")?;
        if !titles.insert(normalize_key(&title)) {
            return Err(AppError::validation("duplicate option title"));
        }
        validate_metadata(&option.metadata)?;
        let rank = match option.rank {
            Some(rank) => rank,
            None => i64::try_from(index).map_err(|_| AppError::validation("rank overflow"))?,
        };
        let option_id = match existing_options.remove(&normalize_key(&title)) {
            Some(option_id) => {
                sqlx::query(
                    "UPDATE product_options SET title = ?, rank = ?, metadata = ? WHERE id = ?",
                )
                .bind(&title)
                .bind(rank)
                .bind(option.metadata.to_json_string())
                .bind(&option_id)
                .execute(&mut **tx)
                .await
                .map_err(database_error)?;
                option_id
            }
            None => {
                let option_id = OptionId::new().as_str().to_owned();
                sqlx::query(
                    "INSERT INTO product_options (id, product_id, title, rank, metadata) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(&option_id)
                .bind(product_id)
                .bind(&title)
                .bind(rank)
                .bind(option.metadata.to_json_string())
                .execute(&mut **tx)
                .await
                .map_err(database_error)?;
                option_id
            }
        };

        let mut values = HashSet::new();
        for (value_index, value) in option.values.iter().enumerate() {
            let value = value.trim();
            validate_length(value, "option value", MAX_OPTION_VALUE_LENGTH)?;
            if !values.insert(normalize_key(value)) {
                return Err(AppError::validation("duplicate option value"));
            }
            let value_rank =
                i64::try_from(value_index).map_err(|_| AppError::validation("rank overflow"))?;
            let key = (option_id.clone(), normalize_key(value));
            let value_id = match existing_values.remove(&key) {
                Some(value_id) => {
                    sqlx::query(
                        "UPDATE product_option_values SET value = ?, rank = ? WHERE id = ?",
                    )
                    .bind(value)
                    .bind(value_rank)
                    .bind(&value_id)
                    .execute(&mut **tx)
                    .await
                    .map_err(database_error)?;
                    value_id
                }
                None => {
                    let value_id = OptionValueId::new().as_str().to_owned();
                    sqlx::query(
                        "INSERT INTO product_option_values \
                         (id, product_option_id, value, rank, metadata) VALUES (?, ?, ?, ?, '{}')",
                    )
                    .bind(&value_id)
                    .bind(&option_id)
                    .bind(value)
                    .bind(value_rank)
                    .execute(&mut **tx)
                    .await
                    .map_err(database_error)?;
                    value_id
                }
            };
            map.insert((normalize_key(&title), normalize_key(value)), value_id);
        }
    }

    // Values are removed first so deleting an option never cascades away a
    // value that another (kept) option still references.
    for value_id in existing_values.values() {
        sqlx::query("DELETE FROM product_option_values WHERE id = ?")
            .bind(value_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    }
    for option_id in existing_options.values() {
        sqlx::query("DELETE FROM product_options WHERE id = ?")
            .bind(option_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    }
    Ok(map)
}

async fn replace_category_links_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    category_ids: &[String],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_category_links WHERE product_id = ?")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    let mut seen = HashSet::new();
    for (rank, category_id) in category_ids.iter().enumerate() {
        if !seen.insert(category_id.clone()) {
            continue;
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM categories WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(category_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
        if count == 0 {
            return Err(AppError::not_found("category", category_id));
        }
        sqlx::query(
            "INSERT INTO product_category_links (product_id, category_id, rank) VALUES (?, ?, ?)",
        )
        .bind(product_id)
        .bind(category_id)
        .bind(i64::try_from(rank).map_err(|_| AppError::validation("rank overflow"))?)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}

async fn replace_collection_links_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    collection_ids: &[String],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_collection_links WHERE product_id = ?")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    let mut seen = HashSet::new();
    for (rank, collection_id) in collection_ids.iter().enumerate() {
        if !seen.insert(collection_id.clone()) {
            continue;
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM collections WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(collection_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(database_error)?;
        if count == 0 {
            return Err(AppError::not_found("collection", collection_id));
        }
        sqlx::query(
            "INSERT INTO product_collection_links (product_id, collection_id, rank) \
             VALUES (?, ?, ?)",
        )
        .bind(product_id)
        .bind(collection_id)
        .bind(i64::try_from(rank).map_err(|_| AppError::validation("rank overflow"))?)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}

async fn replace_tag_links_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    tag_values: &[String],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_tag_links WHERE product_id = ?")
        .bind(product_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    let mut seen = HashSet::new();
    for value in tag_values {
        let value = value.trim();
        validate_length(value, "tag value", MAX_TAG_VALUE_LENGTH)?;
        if !seen.insert(value.to_lowercase()) {
            continue;
        }
        let existing = sqlx::query("SELECT id FROM tags WHERE value = ?")
            .bind(value)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
        let tag_id = match existing {
            Some(row) => column::<String>(&row, "id")?,
            None => {
                let tag_id = TagId::new().as_str().to_owned();
                sqlx::query(
                    "INSERT INTO tags (id, value, metadata, created_at) VALUES (?, ?, '{}', ?)",
                )
                .bind(&tag_id)
                .bind(value)
                .bind(now_timestamp())
                .execute(&mut **tx)
                .await
                .map_err(database_error)?;
                tag_id
            }
        };
        sqlx::query("INSERT OR IGNORE INTO product_tag_links (product_id, tag_id) VALUES (?, ?)")
            .bind(product_id)
            .bind(&tag_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    }
    Ok(())
}

async fn insert_images_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    urls: &[String],
) -> Result<(), AppError> {
    let now = now_timestamp();
    for (index, url) in urls.iter().enumerate() {
        let url = url.trim();
        validate_length(url, "image url", MAX_URL_LENGTH)?;
        let rank = i64::try_from(index).map_err(|_| AppError::validation("rank overflow"))?;
        let image_id = ImageId::new();
        sqlx::query(
            "INSERT INTO product_images \
             (id, product_id, variant_id, url, alt, rank, metadata, created_at) \
             VALUES (?, ?, NULL, ?, NULL, ?, '{}', ?)",
        )
        .bind(image_id.as_str())
        .bind(product_id)
        .bind(url)
        .bind(rank)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}
