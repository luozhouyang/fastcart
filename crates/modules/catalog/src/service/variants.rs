//! Variant authoring and price-set links.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, VariantId, now_timestamp, validate_length, validate_metadata};
use sqlx::Transaction;
use sqlx::sqlite::Sqlite;

use crate::domain::{CreateVariantInput, ProductStatus, ProductVariant, UpdateVariantInput};

use super::shared::*;
/// Full CRUD over the catalog aggregate.
use super::*;
impl CatalogService {
    /// Creates a variant on an existing product.
    pub async fn create_variant(
        &self,
        product_id: &str,
        input: CreateVariantInput,
    ) -> Result<ProductVariant, AppError> {
        self.ensure_product_exists(product_id).await?;
        let option_map = self.option_map_from_pool(product_id).await?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let variant_id = insert_variant_tx(&mut tx, product_id, &input, &option_map).await?;
        sync_search_index_tx(&mut tx, product_id).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_variant(&variant_id).await
    }

    /// Loads a non-deleted variant, including its option-value links.
    pub async fn get_variant(&self, id: &str) -> Result<ProductVariant, AppError> {
        let row = sqlx::query("SELECT * FROM product_variants WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("variant", id))?;
        let mut variant = variant_from_row(&row)?;
        variant.option_value_ids = self.load_variant_option_value_ids(&variant.id).await?;
        Ok(variant)
    }

    /// Loads several non-deleted variants in a fixed number of round trips.
    ///
    /// Cart recalculation and checkout both need every variant on a cart, and
    /// asking one at a time turned a constant amount of work into one query per
    /// line. Ids that do not resolve are simply absent from the result, so the
    /// caller decides whether a missing variant is an error.
    pub async fn get_variants(
        &self,
        ids: &[String],
    ) -> Result<HashMap<String, ProductVariant>, AppError> {
        let mut variants = HashMap::new();
        if ids.is_empty() {
            return Ok(variants);
        }

        // SQLite caps bound parameters per statement, so the ids are read in
        // chunks that comfortably fit.
        for chunk in ids.chunks(SQLITE_PARAMETER_LIMIT) {
            let sql = format!(
                "SELECT * FROM product_variants WHERE deleted_at IS NULL AND id IN ({})",
                sql_markers(chunk.len())
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            for row in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                let variant = variant_from_row(&row)?;
                variants.insert(variant.id.clone(), variant);
            }
        }

        // Option-value links are loaded in one more pass so the result is
        // identical to what `get_variant` returns.
        let loaded: Vec<String> = variants.keys().cloned().collect();
        let mut links: HashMap<String, Vec<String>> = HashMap::new();
        for chunk in loaded.chunks(SQLITE_PARAMETER_LIMIT) {
            let sql = format!(
                "SELECT variant_id, option_value_id FROM product_variant_option_values \
                 WHERE variant_id IN ({}) ORDER BY variant_id ASC, option_value_id ASC",
                sql_markers(chunk.len())
            );
            let mut query = sqlx::query_as::<_, (String, String)>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            for (variant_id, option_value_id) in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                links.entry(variant_id).or_default().push(option_value_id);
            }
        }
        for (id, variant) in &mut variants {
            variant.option_value_ids = links.remove(id).unwrap_or_default();
        }
        Ok(variants)
    }

    /// Loads a non-deleted variant by its unique SKU.
    pub async fn get_variant_by_sku(&self, sku: &str) -> Result<ProductVariant, AppError> {
        let row =
            sqlx::query("SELECT * FROM product_variants WHERE sku = ? AND deleted_at IS NULL")
                .bind(sku)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?
                .ok_or_else(|| AppError::not_found("variant sku", sku))?;
        let mut variant = variant_from_row(&row)?;
        variant.option_value_ids = self.load_variant_option_value_ids(&variant.id).await?;
        Ok(variant)
    }

    /// Partially updates a variant.
    pub async fn update_variant(
        &self,
        id: &str,
        input: UpdateVariantInput,
    ) -> Result<ProductVariant, AppError> {
        let existing = self.get_variant(id).await?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        update_variant_tx(&mut tx, &existing.product_id, id, &input).await?;
        sync_search_index_tx(&mut tx, &existing.product_id).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_variant(id).await
    }

    /// Deletes a variant. Variants referenced by carts or orders are
    /// soft-deleted so historical records remain resolvable.
    pub async fn delete_variant(&self, id: &str) -> Result<(), AppError> {
        let variant = self.get_variant(id).await?;
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        delete_variant_tx(&mut tx, &variant.product_id, id).await?;
        sync_search_index_tx(&mut tx, &variant.product_id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

pub(super) async fn insert_variants_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variants: &[CreateVariantInput],
    option_map: &HashMap<(String, String), String>,
) -> Result<(), AppError> {
    let mut seen_skus = HashSet::new();
    for variant in variants {
        if let Some(sku) = normalize_optional(variant.sku.clone())
            && !seen_skus.insert(sku.to_lowercase())
        {
            return Err(AppError::conflict(format!(
                "duplicate variant sku in input: {sku}"
            )));
        }
        insert_variant_tx(tx, product_id, variant, option_map).await?;
    }
    Ok(())
}

async fn insert_variant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    input: &CreateVariantInput,
    option_map: &HashMap<(String, String), String>,
) -> Result<String, AppError> {
    let title = validate_title(&input.title, "variant title")?;
    validate_metadata(&input.metadata)?;
    validate_weight(input.weight)?;
    let dimensions = input.dimensions.unwrap_or_default();
    validate_dimensions(dimensions)?;
    let sku = normalize_optional(input.sku.clone());
    if let Some(sku) = sku.as_deref() {
        validate_length(sku, "variant sku", MAX_SKU_LENGTH)?;
        ensure_sku_available_tx(tx, sku, None).await?;
    }
    let barcode = normalize_optional(input.barcode.clone());
    let tax_category_id = normalize_optional(input.tax_category_id.clone());
    let status = input.status.unwrap_or(ProductStatus::Active);
    let option_value_ids = resolve_variant_option_ids(input, option_map)?;
    let now = now_timestamp();
    let id = VariantId::new().as_str().to_owned();

    sqlx::query(
        "INSERT INTO product_variants \
         (id, product_id, title, sku, barcode, status, weight, length, height, width, \
          manage_inventory, allow_backorder, tax_category_id, metadata, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(product_id)
    .bind(&title)
    .bind(&sku)
    .bind(&barcode)
    .bind(status.as_str())
    .bind(input.weight)
    .bind(dimensions.length)
    .bind(dimensions.height)
    .bind(dimensions.width)
    .bind(bool_to_int(input.manage_inventory.unwrap_or(true)))
    .bind(bool_to_int(input.allow_backorder.unwrap_or(false)))
    .bind(&tax_category_id)
    .bind(input.metadata.to_json_string())
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|error| unique_or_database(error, "variant sku already exists"))?;

    replace_variant_option_values_tx(tx, &id, &option_value_ids).await?;
    Ok(id)
}

async fn update_variant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variant_id: &str,
    input: &UpdateVariantInput,
) -> Result<(), AppError> {
    let row = sqlx::query(
        "SELECT * FROM product_variants \
         WHERE id = ? AND product_id = ? AND deleted_at IS NULL",
    )
    .bind(variant_id)
    .bind(product_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?
    .ok_or_else(|| AppError::not_found("variant", variant_id))?;
    let existing = variant_from_row(&row)?;

    let title = match input.title.as_deref() {
        Some(value) => validate_title(value, "variant title")?,
        None => existing.title.clone(),
    };
    let sku = match input.sku.as_deref() {
        Some(value) => normalize_optional(Some(value.to_owned())),
        None => existing.sku.clone(),
    };
    if sku != existing.sku
        && let Some(sku) = sku.as_deref()
    {
        validate_length(sku, "variant sku", MAX_SKU_LENGTH)?;
        ensure_sku_available_tx(tx, sku, Some(variant_id)).await?;
    }
    let barcode = match input.barcode.as_deref() {
        Some(value) => normalize_optional(Some(value.to_owned())),
        None => existing.barcode.clone(),
    };
    let tax_category_id = match input.tax_category_id.as_deref() {
        Some(value) => normalize_optional(Some(value.to_owned())),
        None => existing.tax_category_id.clone(),
    };
    let status = input.status.unwrap_or(existing.status);
    let weight = input.weight.or(existing.weight);
    let dimensions = input.dimensions.unwrap_or(existing.dimensions);
    validate_dimensions(dimensions)?;
    let metadata = match input.metadata.as_ref() {
        Some(metadata) => {
            validate_metadata(metadata)?;
            metadata.clone()
        }
        None => existing.metadata.clone(),
    };

    sqlx::query(
        "UPDATE product_variants SET title = ?, sku = ?, barcode = ?, status = ?, weight = ?, \
         length = ?, height = ?, width = ?, manage_inventory = ?, allow_backorder = ?, \
         tax_category_id = ?, metadata = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&title)
    .bind(&sku)
    .bind(&barcode)
    .bind(status.as_str())
    .bind(weight)
    .bind(dimensions.length)
    .bind(dimensions.height)
    .bind(dimensions.width)
    .bind(bool_to_int(
        input.manage_inventory.unwrap_or(existing.manage_inventory),
    ))
    .bind(bool_to_int(
        input.allow_backorder.unwrap_or(existing.allow_backorder),
    ))
    .bind(&tax_category_id)
    .bind(metadata.to_json_string())
    .bind(now_timestamp())
    .bind(variant_id)
    .execute(&mut **tx)
    .await
    .map_err(|error| unique_or_database(error, "variant sku already exists"))?;

    if let Some(option_value_ids) = input.option_value_ids.as_ref() {
        ensure_option_values_belong_tx(tx, product_id, option_value_ids).await?;
        replace_variant_option_values_tx(tx, variant_id, option_value_ids).await?;
    }
    Ok(())
}

pub(super) async fn replace_variants_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variants: &[UpdateVariantInput],
    option_map: &HashMap<(String, String), String>,
) -> Result<(), AppError> {
    let existing_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM product_variants WHERE product_id = ? AND deleted_at IS NULL",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(database_error)?;

    let mut kept = HashSet::new();
    for variant in variants {
        match variant.id.as_deref() {
            Some(variant_id) => {
                if !existing_ids.iter().any(|id| id == variant_id) {
                    return Err(AppError::not_found("variant", variant_id));
                }
                update_variant_tx(tx, product_id, variant_id, variant).await?;
                kept.insert(variant_id.to_owned());
            }
            None => {
                let input = create_input_from_update(variant);
                let id = insert_variant_tx(tx, product_id, &input, option_map).await?;
                kept.insert(id);
            }
        }
    }

    for variant_id in existing_ids {
        if !kept.contains(&variant_id) {
            delete_variant_tx(tx, product_id, &variant_id).await?;
        }
    }
    Ok(())
}

fn create_input_from_update(input: &UpdateVariantInput) -> CreateVariantInput {
    CreateVariantInput {
        title: input.title.clone().unwrap_or_default(),
        sku: input.sku.clone(),
        barcode: input.barcode.clone(),
        status: input.status,
        weight: input.weight,
        dimensions: input.dimensions,
        manage_inventory: input.manage_inventory,
        allow_backorder: input.allow_backorder,
        tax_category_id: input.tax_category_id.clone(),
        metadata: input.metadata.clone().unwrap_or_default(),
        option_value_ids: input.option_value_ids.clone().unwrap_or_default(),
        option_values: Vec::new(),
    }
}

async fn delete_variant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variant_id: &str,
) -> Result<(), AppError> {
    let referenced = variant_is_referenced_tx(tx, variant_id).await?;
    if referenced {
        sqlx::query("UPDATE product_variants SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(now_timestamp())
            .bind(variant_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    } else {
        sqlx::query("DELETE FROM product_variants WHERE id = ? AND product_id = ?")
            .bind(variant_id)
            .bind(product_id)
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
    }
    Ok(())
}

async fn variant_is_referenced_tx(
    tx: &mut Transaction<'_, Sqlite>,
    variant_id: &str,
) -> Result<bool, AppError> {
    let cart_items: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cart_items WHERE variant_id = ?")
            .bind(variant_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?;
    let order_items: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM order_items WHERE variant_id = ?")
            .bind(variant_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?;
    Ok(cart_items + order_items > 0)
}

async fn ensure_sku_available_tx(
    tx: &mut Transaction<'_, Sqlite>,
    sku: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let count: i64 = match exclude_id {
        Some(exclude_id) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE sku = ? AND id <> ?")
                .bind(sku)
                .bind(exclude_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(database_error)?
        }
        None => sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE sku = ?")
            .bind(sku)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?,
    };
    if count > 0 {
        return Err(AppError::conflict(format!(
            "variant sku already exists: {sku}"
        )));
    }
    Ok(())
}

fn resolve_variant_option_ids(
    input: &CreateVariantInput,
    option_map: &HashMap<(String, String), String>,
) -> Result<Vec<String>, AppError> {
    let allowed: HashSet<&str> = option_map.values().map(String::as_str).collect();
    let mut ids: Vec<String> = Vec::new();
    for option_value_id in &input.option_value_ids {
        if !allowed.contains(option_value_id.as_str()) {
            return Err(AppError::validation(
                "variant option value does not belong to the product",
            ));
        }
        if !ids.contains(option_value_id) {
            ids.push(option_value_id.clone());
        }
    }
    for selection in &input.option_values {
        let key = (
            normalize_key(&selection.option_title),
            normalize_key(&selection.value),
        );
        let option_value_id = option_map.get(&key).ok_or_else(|| {
            AppError::validation(format!(
                "unknown option value: {} / {}",
                selection.option_title, selection.value
            ))
        })?;
        if !ids.contains(option_value_id) {
            ids.push(option_value_id.clone());
        }
    }
    Ok(ids)
}

async fn ensure_option_values_belong_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    option_value_ids: &[String],
) -> Result<(), AppError> {
    if option_value_ids.is_empty() {
        return Ok(());
    }
    let markers = sql_markers(option_value_ids.len());
    let sql = format!(
        "SELECT COUNT(*) FROM product_option_values v \
         JOIN product_options o ON o.id = v.product_option_id \
         WHERE o.product_id = ? AND v.id IN ({markers})"
    );
    let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(product_id);
    for option_value_id in option_value_ids {
        query = query.bind(option_value_id);
    }
    let count = query.fetch_one(&mut **tx).await.map_err(database_error)?;
    if count != i64::try_from(option_value_ids.len()).unwrap_or(i64::MAX) {
        return Err(AppError::validation(
            "variant option value does not belong to the product",
        ));
    }
    Ok(())
}

async fn replace_variant_option_values_tx(
    tx: &mut Transaction<'_, Sqlite>,
    variant_id: &str,
    option_value_ids: &[String],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM product_variant_option_values WHERE variant_id = ?")
        .bind(variant_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    for option_value_id in option_value_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO product_variant_option_values \
             (variant_id, option_value_id) VALUES (?, ?)",
        )
        .bind(variant_id)
        .bind(option_value_id)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}

impl CatalogService {
    /// Links a price set to a variant inside the caller's transaction.
    ///
    /// The link table is owned by the catalog module. Pricing calls this while
    /// creating the price set so the two rows commit or roll back together.
    pub async fn attach_price_set(
        transaction: &mut Transaction<'_, Sqlite>,
        variant_id: &str,
        price_set_id: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO product_variant_price_set (variant_id, price_set_id) VALUES (?, ?)",
        )
        .bind(variant_id)
        .bind(price_set_id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        Ok(())
    }
}
