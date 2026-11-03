//! SQLite-backed catalog service.

use std::collections::{HashMap, HashSet};

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CategoryId, CollectionId, ImageId, Metadata, OptionId, OptionValueId, PageResult,
    Pagination, ProductId, TagId, VariantId, now_timestamp, slugify, validate_handle,
    validate_length, validate_metadata,
};
use sqlx::Row;
use sqlx::Transaction;
use sqlx::sqlite::{Sqlite, SqliteRow};

use crate::domain::{
    Category, Collection, CreateCategoryInput, CreateCollectionInput, CreateImageInput,
    CreateOptionInput, CreateProductInput, CreateVariantInput, Dimensions, Product, ProductFilter,
    ProductImage, ProductOption, ProductOptionValue, ProductStatus, ProductVariant, Tag,
    UpdateCategoryInput, UpdateCollectionInput, UpdateProductInput, UpdateVariantInput,
};

/// Maximum product, variant, option or category title length.
pub const MAX_TITLE_LENGTH: usize = 200;
/// Maximum option value length.
pub const MAX_OPTION_VALUE_LENGTH: usize = 200;
/// Maximum SKU length.
pub const MAX_SKU_LENGTH: usize = 120;
/// Maximum tag value length.
pub const MAX_TAG_VALUE_LENGTH: usize = 100;
/// Maximum image URL length.
pub const MAX_URL_LENGTH: usize = 2048;

const PRODUCT_COLUMNS: &str = "p.id, p.title, p.subtitle, p.description, p.handle, p.status, \
     p.thumbnail, p.weight, p.length, p.height, p.width, p.metadata, p.created_at, p.updated_at";

/// Full CRUD over the catalog aggregate.
#[derive(Clone, Debug)]
pub struct CatalogService {
    db: StoreDatabase,
}

impl CatalogService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    // ---------------------------------------------------------------------
    // Products
    // ---------------------------------------------------------------------

    /// Creates a product and every nested aggregate in one transaction.
    pub async fn create_product(&self, input: CreateProductInput) -> Result<Product, AppError> {
        let title = validate_title(&input.title, "product title")?;
        let handle = resolve_handle(input.handle.as_deref(), &title, "product handle")?;
        validate_metadata(&input.metadata)?;
        validate_weight(input.weight)?;
        let dimensions = input.dimensions.unwrap_or_default();
        validate_dimensions(dimensions)?;
        for url in &input.images {
            validate_length(url, "image url", MAX_URL_LENGTH)?;
        }

        let product_id = ProductId::new().as_str().to_owned();
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        let existing_handles: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE handle = ?")
                .bind(&handle)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if existing_handles > 0 {
            return Err(AppError::conflict(format!(
                "product handle already exists: {handle}"
            )));
        }

        sqlx::query(
            "INSERT INTO products \
             (id, title, subtitle, description, handle, status, thumbnail, weight, length, height, \
              width, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&product_id)
        .bind(&title)
        .bind(normalize_optional(input.subtitle))
        .bind(normalize_optional(input.description))
        .bind(&handle)
        .bind(ProductStatus::Draft.as_str())
        .bind(normalize_optional(input.thumbnail))
        .bind(input.weight)
        .bind(dimensions.length)
        .bind(dimensions.height)
        .bind(dimensions.width)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "product handle already exists"))?;

        let option_map = insert_options_tx(&mut tx, &product_id, &input.options).await?;
        insert_variants_tx(&mut tx, &product_id, &input.variants, &option_map).await?;
        replace_category_links_tx(&mut tx, &product_id, &input.category_ids).await?;
        replace_collection_links_tx(&mut tx, &product_id, &input.collection_ids).await?;
        replace_tag_links_tx(&mut tx, &product_id, &input.tag_values).await?;
        insert_images_tx(&mut tx, &product_id, &input.images).await?;
        sync_search_index_tx(&mut tx, &product_id).await?;

        tx.commit().await.map_err(database_error)?;
        self.get_product(&product_id).await
    }

    /// Loads a product aggregate, including all active variants, options,
    /// images and organizational links.
    pub async fn get_product(&self, id: &str) -> Result<Product, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p WHERE p.id = ? AND p.deleted_at IS NULL"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("product", id))?;
        let mut product = product_from_row(&row)?;
        self.hydrate_products(std::slice::from_mut(&mut product))
            .await?;
        Ok(product)
    }

    /// Loads a non-deleted product by its unique handle.
    pub async fn get_product_by_handle(&self, handle: &str) -> Result<Product, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p WHERE p.handle = ? AND p.deleted_at IS NULL"
        ))
        .bind(handle)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("product handle", handle))?;
        let mut product = product_from_row(&row)?;
        self.hydrate_products(std::slice::from_mut(&mut product))
            .await?;
        Ok(product)
    }

    /// Partially updates a product, including variant and link replacement.
    pub async fn update_product(
        &self,
        id: &str,
        input: UpdateProductInput,
    ) -> Result<Product, AppError> {
        let existing = self.get_product(id).await?;

        let title = match input.title.as_deref() {
            Some(value) => validate_title(value, "product title")?,
            None => existing.title.clone(),
        };
        let handle = match input.handle.as_deref() {
            Some(value) => {
                let handle = value.trim().to_owned();
                validate_handle(&handle, "product handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        let status = match input.status {
            Some(status) => {
                if existing.status == ProductStatus::Archived && status != ProductStatus::Archived {
                    return Err(AppError::conflict("archived products cannot change status"));
                }
                status
            }
            None => existing.status,
        };
        let weight = input.weight.or(existing.weight);
        let dimensions = input.dimensions.unwrap_or(existing.dimensions);
        validate_dimensions(dimensions)?;
        let subtitle = match input.subtitle {
            Some(value) => normalize_optional(Some(value)),
            None => existing.subtitle.clone(),
        };
        let description = match input.description {
            Some(value) => normalize_optional(Some(value)),
            None => existing.description.clone(),
        };
        let thumbnail = match input.thumbnail {
            Some(value) => normalize_optional(Some(value)),
            None => existing.thumbnail.clone(),
        };
        let metadata = match input.metadata {
            Some(metadata) => {
                validate_metadata(&metadata)?;
                metadata
            }
            None => existing.metadata.clone(),
        };
        if let Some(images) = input.images.as_ref() {
            for url in images {
                validate_length(url, "image url", MAX_URL_LENGTH)?;
            }
        }

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        if handle != existing.handle {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE handle = ? AND id <> ?")
                    .bind(&handle)
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(database_error)?;
            if count > 0 {
                return Err(AppError::conflict(format!(
                    "product handle already exists: {handle}"
                )));
            }
        }

        sqlx::query(
            "UPDATE products SET title = ?, subtitle = ?, description = ?, handle = ?, status = ?, \
             thumbnail = ?, weight = ?, length = ?, height = ?, width = ?, metadata = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(&title)
        .bind(&subtitle)
        .bind(&description)
        .bind(&handle)
        .bind(status.as_str())
        .bind(&thumbnail)
        .bind(weight)
        .bind(dimensions.length)
        .bind(dimensions.height)
        .bind(dimensions.width)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "product handle already exists"))?;

        let option_map = match input.options.as_ref() {
            Some(options) => {
                sqlx::query("DELETE FROM product_options WHERE product_id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(database_error)?;
                insert_options_tx(&mut tx, id, options).await?
            }
            None => option_map_from_tx(&mut tx, id).await?,
        };

        if let Some(variants) = input.variants.as_ref() {
            replace_variants_tx(&mut tx, id, variants, &option_map).await?;
        }
        if let Some(category_ids) = input.category_ids.as_ref() {
            replace_category_links_tx(&mut tx, id, category_ids).await?;
        }
        if let Some(collection_ids) = input.collection_ids.as_ref() {
            replace_collection_links_tx(&mut tx, id, collection_ids).await?;
        }
        if let Some(tag_values) = input.tag_values.as_ref() {
            replace_tag_links_tx(&mut tx, id, tag_values).await?;
        }
        if let Some(images) = input.images.as_ref() {
            sqlx::query("DELETE FROM product_images WHERE product_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            insert_images_tx(&mut tx, id, images).await?;
        }
        sync_search_index_tx(&mut tx, id).await?;

        tx.commit().await.map_err(database_error)?;
        self.get_product(id).await
    }

    /// Soft-deletes a product. Variants are retained for historical records.
    pub async fn delete_product(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE products SET deleted_at = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now_timestamp())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("product", id));
        }
        sync_search_index_tx(&mut tx, id).await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Lists products using administrative filters.
    pub async fn list_products(
        &self,
        filter: ProductFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let (where_clause, binds) = product_filter_clause(&filter);

        let count_sql = format!("SELECT COUNT(*) FROM products p WHERE {where_clause}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        for bind in &binds {
            count_query = count_query.bind(bind);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p WHERE {where_clause} \
             ORDER BY p.created_at ASC, p.id ASC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query(&list_sql);
        for bind in &binds {
            list_query = list_query.bind(bind);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut products = Vec::with_capacity(rows.len());
        for row in &rows {
            products.push(product_from_row(row)?);
        }
        self.hydrate_products(&mut products).await?;
        Ok(PageResult::new(products, count, pagination))
    }

    /// Publishes a draft or active product.
    pub async fn publish_product(&self, id: &str) -> Result<Product, AppError> {
        let existing = self.get_product(id).await?;
        if !existing.status.can_publish() {
            return Err(AppError::conflict("archived products cannot be published"));
        }
        if existing.status == ProductStatus::Active {
            return Ok(existing);
        }
        self.set_product_status(id, ProductStatus::Active).await
    }

    /// Archives a product. Archived products are no longer purchasable.
    pub async fn archive_product(&self, id: &str) -> Result<Product, AppError> {
        let existing = self.get_product(id).await?;
        if existing.status == ProductStatus::Archived {
            return Ok(existing);
        }
        self.set_product_status(id, ProductStatus::Archived).await
    }

    /// Lists active, non-deleted products for storefronts.
    pub async fn list_published(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM products p WHERE p.deleted_at IS NULL AND p.status = 'active'",
        )
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;

        let rows = sqlx::query(&format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p \
             WHERE p.deleted_at IS NULL AND p.status = 'active' \
             ORDER BY p.created_at ASC, p.id ASC LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let mut products = Vec::with_capacity(rows.len());
        for row in &rows {
            products.push(product_from_row(row)?);
        }
        self.hydrate_products(&mut products).await?;
        Ok(PageResult::new(products, count, pagination))
    }

    /// Loads an active, non-deleted product by handle for storefronts.
    pub async fn get_published_by_handle(&self, handle: &str) -> Result<Product, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p \
             WHERE p.handle = ? AND p.deleted_at IS NULL AND p.status = 'active'"
        ))
        .bind(handle)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("published product handle", handle))?;
        let mut product = product_from_row(&row)?;
        self.hydrate_products(std::slice::from_mut(&mut product))
            .await?;
        Ok(product)
    }

    /// Loads an active, non-deleted product by handle when it is published to
    /// at least one of the supplied sales channels.
    ///
    /// An empty channel list applies no channel filter.
    pub async fn get_published_by_handle_in_channels(
        &self,
        handle: &str,
        sales_channel_ids: &[String],
    ) -> Result<Product, AppError> {
        if sales_channel_ids.is_empty() {
            return self.get_published_by_handle(handle).await;
        }
        let placeholders = sql_markers(sales_channel_ids.len());
        let sql = format!(
            "SELECT {PRODUCT_COLUMNS} FROM products p \
             WHERE p.handle = ? AND p.deleted_at IS NULL AND p.status = 'active' \
             AND EXISTS (SELECT 1 FROM product_sales_channels sc \
                         WHERE sc.product_id = p.id AND sc.sales_channel_id IN ({placeholders}))"
        );
        let mut query = sqlx::query(&sql).bind(handle);
        for channel_id in sales_channel_ids {
            query = query.bind(channel_id);
        }
        let row = query
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("published product handle", handle))?;
        let mut product = product_from_row(&row)?;
        self.hydrate_products(std::slice::from_mut(&mut product))
            .await?;
        Ok(product)
    }

    // ---------------------------------------------------------------------
    // Sales channels
    // ---------------------------------------------------------------------

    /// Replaces the sales channels a product is published to.
    ///
    /// Every channel must exist before any link is changed, and the replace is
    /// performed in a single transaction.
    pub async fn set_product_sales_channels(
        &self,
        product_id: &str,
        channel_ids: &[String],
    ) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ? AND deleted_at IS NULL")
                .bind(product_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if exists == 0 {
            return Err(AppError::not_found("product", product_id));
        }

        let mut seen = HashSet::new();
        for channel_id in channel_ids {
            if !seen.insert(channel_id) {
                continue;
            }
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels WHERE id = ?")
                .bind(channel_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
            if count == 0 {
                return Err(AppError::not_found("sales channel", channel_id));
            }
        }

        sqlx::query("DELETE FROM product_sales_channels WHERE product_id = ?")
            .bind(product_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        for channel_id in seen {
            sqlx::query(
                "INSERT INTO product_sales_channels (product_id, sales_channel_id) VALUES (?, ?)",
            )
            .bind(product_id)
            .bind(channel_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Lists the sales channels a product is published to.
    pub async fn list_product_sales_channel_ids(
        &self,
        product_id: &str,
    ) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar(
            "SELECT sales_channel_id FROM product_sales_channels \
             WHERE product_id = ? ORDER BY sales_channel_id ASC",
        )
        .bind(product_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)
    }

    async fn set_product_status(
        &self,
        id: &str,
        status: ProductStatus,
    ) -> Result<Product, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query(
            "UPDATE products SET status = ?, updated_at = ? \
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(status.as_str())
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("product", id));
        }
        sync_search_index_tx(&mut tx, id).await?;
        tx.commit().await.map_err(database_error)?;
        self.get_product(id).await
    }

    // ---------------------------------------------------------------------
    // Variants
    // ---------------------------------------------------------------------

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

    // ---------------------------------------------------------------------
    // Categories
    // ---------------------------------------------------------------------

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
        let existing = self.get_category(id).await?;
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
            self.ensure_category_handle_available(&handle, Some(id))
                .await?;
        }
        let parent_id = match input.parent_id {
            Some(value) => {
                let parent_id = normalize_optional(Some(value));
                if let Some(parent_id) = parent_id.as_deref() {
                    self.ensure_category_exists(parent_id).await?;
                    self.ensure_no_category_cycle(id, parent_id).await?;
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

        sqlx::query(
            "UPDATE categories SET parent_id = ?, title = ?, handle = ?, description = ?, \
             is_active = ?, is_internal = ?, rank = ?, metadata = ?, updated_at = ? WHERE id = ?",
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
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "category handle already exists"))?;
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
        self.get_category(id).await?;
        let children: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM categories WHERE parent_id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(self.db.pool())
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
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        if linked > 0 {
            return Err(AppError::conflict(
                "category has linked products and cannot be deleted",
            ));
        }
        sqlx::query("UPDATE categories SET deleted_at = ?, updated_at = ? WHERE id = ?")
            .bind(now_timestamp())
            .bind(now_timestamp())
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
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

    async fn ensure_no_category_cycle(
        &self,
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
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?
                .map(|row| column::<Option<String>>(&row, "parent_id"))
                .transpose()?
                .flatten();
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Collections
    // ---------------------------------------------------------------------

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

    // ---------------------------------------------------------------------
    // Tags
    // ---------------------------------------------------------------------

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
        self.ensure_product_exists(product_id).await?;
        let tag = self.create_tag(value).await?;
        sqlx::query("INSERT OR IGNORE INTO product_tag_links (product_id, tag_id) VALUES (?, ?)")
            .bind(product_id)
            .bind(&tag.id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
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

    // ---------------------------------------------------------------------
    // Images
    // ---------------------------------------------------------------------

    /// Adds an image to a product.
    pub async fn add_image(
        &self,
        product_id: &str,
        input: CreateImageInput,
    ) -> Result<ProductImage, AppError> {
        let url = input.url.trim();
        validate_length(url, "image url", MAX_URL_LENGTH)?;
        validate_metadata(&input.metadata)?;
        self.ensure_product_exists(product_id).await?;
        if let Some(variant_id) = input.variant_id.as_deref() {
            self.ensure_variant_belongs(product_id, variant_id).await?;
        }
        let rank = match input.rank {
            Some(rank) => rank,
            None => sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(rank), -1) + 1 FROM product_images WHERE product_id = ?",
            )
            .bind(product_id)
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?,
        };
        let id = ImageId::new().as_str().to_owned();
        sqlx::query(
            "INSERT INTO product_images \
             (id, product_id, variant_id, url, alt, rank, metadata, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(product_id)
        .bind(&input.variant_id)
        .bind(url)
        .bind(normalize_optional(input.alt))
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .bind(now_timestamp())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_image(&id).await
    }

    /// Loads a product image.
    pub async fn get_image(&self, id: &str) -> Result<ProductImage, AppError> {
        let row = sqlx::query("SELECT * FROM product_images WHERE id = ?")
            .bind(id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("image", id))?;
        image_from_row(&row)
    }

    /// Lists a product's images ordered by rank.
    pub async fn list_images(&self, product_id: &str) -> Result<Vec<ProductImage>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM product_images WHERE product_id = ? \
             ORDER BY rank ASC, created_at ASC, id ASC",
        )
        .bind(product_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let mut images = Vec::with_capacity(rows.len());
        for row in &rows {
            images.push(image_from_row(row)?);
        }
        Ok(images)
    }

    /// Removes a product image.
    pub async fn remove_image(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM product_images WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("image", id));
        }
        Ok(())
    }

    /// Reorders a product's images according to the supplied identifier order.
    pub async fn reorder_images(
        &self,
        product_id: &str,
        image_ids: &[String],
    ) -> Result<Vec<ProductImage>, AppError> {
        let existing = self.list_images(product_id).await?;
        let existing_ids: HashSet<&str> = existing.iter().map(|image| image.id.as_str()).collect();
        for image_id in image_ids {
            if !existing_ids.contains(image_id.as_str()) {
                return Err(AppError::not_found("image", image_id));
            }
        }
        for (rank, image_id) in image_ids.iter().enumerate() {
            sqlx::query("UPDATE product_images SET rank = ? WHERE id = ? AND product_id = ?")
                .bind(i64::try_from(rank).map_err(|_| AppError::validation("rank overflow"))?)
                .bind(image_id)
                .bind(product_id)
                .execute(self.db.pool())
                .await
                .map_err(database_error)?;
        }
        self.list_images(product_id).await
    }

    // ---------------------------------------------------------------------
    // Search
    // ---------------------------------------------------------------------

    /// Searches active, non-deleted products for storefronts using FTS5.
    pub async fn search_products(
        &self,
        query: &str,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        let filter = ProductFilter {
            status: Some(ProductStatus::Active),
            ..ProductFilter::default()
        };
        self.search_products_with_filter(query, filter, pagination)
            .await
    }

    /// Searches all non-deleted products for administration using FTS5.
    pub async fn search_products_admin(
        &self,
        query: &str,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        self.search_products_with_filter(query, ProductFilter::default(), pagination)
            .await
    }

    /// Searches products using FTS5 plus the supplied catalog filters.
    ///
    /// This is the storefront entry point when sales-channel scoping (or any
    /// other structural filter) must be combined with full-text search.
    pub async fn search_products_with_filter(
        &self,
        query: &str,
        filter: ProductFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let Some(match_expression) = fts_query(query) else {
            return Ok(PageResult::new(Vec::new(), 0, pagination));
        };
        let (where_clause, binds) = product_filter_clause(&filter);
        let count_sql = format!(
            "SELECT COUNT(*) FROM product_search \
             JOIN products p ON p.id = product_search.product_id \
             WHERE product_search MATCH ? AND {where_clause}"
        );
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql).bind(&match_expression);
        for bind in &binds {
            count_query = count_query.bind(bind);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {PRODUCT_COLUMNS} FROM product_search \
             JOIN products p ON p.id = product_search.product_id \
             WHERE product_search MATCH ? AND {where_clause} \
             ORDER BY p.created_at ASC, p.id ASC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query(&list_sql).bind(&match_expression);
        for bind in &binds {
            list_query = list_query.bind(bind);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut products = Vec::with_capacity(rows.len());
        for row in &rows {
            products.push(product_from_row(row)?);
        }
        self.hydrate_products(&mut products).await?;
        Ok(PageResult::new(products, count, pagination))
    }

    /// Rebuilds the FTS index from the product and variant tables.
    pub async fn reindex_search(&self) -> Result<u64, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query("DELETE FROM product_search")
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        let result = sqlx::query(
            "INSERT INTO product_search (product_id, title, description, sku, handle) \
             SELECT p.id, p.title, COALESCE(p.description, ''), \
                    COALESCE((SELECT group_concat(v.sku, ' ') FROM product_variants v \
                              WHERE v.product_id = p.id AND v.sku IS NOT NULL \
                              AND v.deleted_at IS NULL), ''), \
                    p.handle \
             FROM products p WHERE p.deleted_at IS NULL",
        )
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(result.rows_affected())
    }

    // ---------------------------------------------------------------------
    // Aggregates and shared helpers
    // ---------------------------------------------------------------------

    async fn hydrate_products(&self, products: &mut [Product]) -> Result<(), AppError> {
        if products.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = products.iter().map(|product| product.id.clone()).collect();
        let placeholders = sql_markers(ids.len());

        let mut variants: HashMap<String, Vec<ProductVariant>> = HashMap::new();
        let sql = format!(
            "SELECT * FROM product_variants WHERE deleted_at IS NULL \
             AND product_id IN ({placeholders}) ORDER BY created_at ASC, id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let variant = variant_from_row(&row)?;
            variants
                .entry(variant.product_id.clone())
                .or_default()
                .push(variant);
        }

        let variant_ids: Vec<String> = variants
            .values()
            .flatten()
            .map(|variant| variant.id.clone())
            .collect();
        if !variant_ids.is_empty() {
            let placeholders = sql_markers(variant_ids.len());
            let sql = format!(
                "SELECT variant_id, option_value_id FROM product_variant_option_values \
                 WHERE variant_id IN ({placeholders}) ORDER BY option_value_id ASC"
            );
            let mut query = sqlx::query(&sql);
            for id in &variant_ids {
                query = query.bind(id);
            }
            let mut links: HashMap<String, Vec<String>> = HashMap::new();
            for row in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                let variant_id: String = column(&row, "variant_id")?;
                let option_value_id: String = column(&row, "option_value_id")?;
                links.entry(variant_id).or_default().push(option_value_id);
            }
            for variant in variants.values_mut().flatten() {
                if let Some(option_value_ids) = links.remove(&variant.id) {
                    variant.option_value_ids = option_value_ids;
                }
            }
        }

        let mut options: HashMap<String, Vec<ProductOption>> = HashMap::new();
        let sql = format!(
            "SELECT * FROM product_options WHERE product_id IN ({placeholders}) \
             ORDER BY rank ASC, id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let option = option_from_row(&row)?;
            options
                .entry(option.product_id.clone())
                .or_default()
                .push(option);
        }

        let option_ids: Vec<String> = options
            .values()
            .flatten()
            .map(|option| option.id.clone())
            .collect();
        if !option_ids.is_empty() {
            let placeholders = sql_markers(option_ids.len());
            let sql = format!(
                "SELECT * FROM product_option_values WHERE product_option_id IN ({placeholders}) \
                 ORDER BY rank ASC, id ASC"
            );
            let mut query = sqlx::query(&sql);
            for id in &option_ids {
                query = query.bind(id);
            }
            let mut values: HashMap<String, Vec<ProductOptionValue>> = HashMap::new();
            for row in query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?
            {
                let value = option_value_from_row(&row)?;
                values
                    .entry(value.option_id.clone())
                    .or_default()
                    .push(value);
            }
            for option in options.values_mut().flatten() {
                if let Some(option_values) = values.remove(&option.id) {
                    option.values = option_values;
                }
            }
        }

        let mut images: HashMap<String, Vec<ProductImage>> = HashMap::new();
        let sql = format!(
            "SELECT * FROM product_images WHERE product_id IN ({placeholders}) \
             ORDER BY rank ASC, created_at ASC, id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let image = image_from_row(&row)?;
            images
                .entry(image.product_id.clone())
                .or_default()
                .push(image);
        }

        let mut categories: HashMap<String, Vec<Category>> = HashMap::new();
        let sql = format!(
            "SELECT c.*, l.product_id AS link_product_id \
             FROM categories c JOIN product_category_links l ON l.category_id = c.id \
             WHERE l.product_id IN ({placeholders}) AND c.deleted_at IS NULL \
             ORDER BY l.rank ASC, c.title ASC, c.id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let product_id: String = column(&row, "link_product_id")?;
            categories
                .entry(product_id)
                .or_default()
                .push(category_from_row(&row)?);
        }

        let mut collections: HashMap<String, Vec<Collection>> = HashMap::new();
        let sql = format!(
            "SELECT c.*, l.product_id AS link_product_id \
             FROM collections c JOIN product_collection_links l ON l.collection_id = c.id \
             WHERE l.product_id IN ({placeholders}) AND c.deleted_at IS NULL \
             ORDER BY l.rank ASC, c.title ASC, c.id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let product_id: String = column(&row, "link_product_id")?;
            collections
                .entry(product_id)
                .or_default()
                .push(collection_from_row(&row)?);
        }

        let mut tags: HashMap<String, Vec<Tag>> = HashMap::new();
        let sql = format!(
            "SELECT t.*, l.product_id AS link_product_id \
             FROM tags t JOIN product_tag_links l ON l.tag_id = t.id \
             WHERE l.product_id IN ({placeholders}) ORDER BY t.value ASC, t.id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let product_id: String = column(&row, "link_product_id")?;
            tags.entry(product_id)
                .or_default()
                .push(tag_from_row(&row)?);
        }

        let mut sales_channels: HashMap<String, Vec<String>> = HashMap::new();
        let sql = format!(
            "SELECT product_id, sales_channel_id FROM product_sales_channels \
             WHERE product_id IN ({placeholders}) ORDER BY sales_channel_id ASC"
        );
        let mut query = sqlx::query(&sql);
        for id in &ids {
            query = query.bind(id);
        }
        for row in query
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?
        {
            let product_id: String = column(&row, "product_id")?;
            let sales_channel_id: String = column(&row, "sales_channel_id")?;
            sales_channels
                .entry(product_id)
                .or_default()
                .push(sales_channel_id);
        }

        for product in products.iter_mut() {
            product.variants = variants.remove(&product.id).unwrap_or_default();
            product.options = options.remove(&product.id).unwrap_or_default();
            product.images = images.remove(&product.id).unwrap_or_default();
            product.categories = categories.remove(&product.id).unwrap_or_default();
            product.collections = collections.remove(&product.id).unwrap_or_default();
            product.tags = tags.remove(&product.id).unwrap_or_default();
            product.sales_channel_ids = sales_channels.remove(&product.id).unwrap_or_default();
        }
        Ok(())
    }

    async fn load_variant_option_value_ids(
        &self,
        variant_id: &str,
    ) -> Result<Vec<String>, AppError> {
        sqlx::query_scalar(
            "SELECT option_value_id FROM product_variant_option_values \
             WHERE variant_id = ? ORDER BY option_value_id ASC",
        )
        .bind(variant_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)
    }

    async fn ensure_product_exists(&self, id: &str) -> Result<(), AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM products WHERE id = ? AND deleted_at IS NULL")
                .bind(id)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        if count == 0 {
            return Err(AppError::not_found("product", id));
        }
        Ok(())
    }

    async fn ensure_variant_belongs(
        &self,
        product_id: &str,
        variant_id: &str,
    ) -> Result<(), AppError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM product_variants \
             WHERE id = ? AND product_id = ? AND deleted_at IS NULL",
        )
        .bind(variant_id)
        .bind(product_id)
        .fetch_one(self.db.pool())
        .await
        .map_err(database_error)?;
        if count == 0 {
            return Err(AppError::not_found("variant", variant_id));
        }
        Ok(())
    }

    async fn option_map_from_pool(
        &self,
        product_id: &str,
    ) -> Result<HashMap<(String, String), String>, AppError> {
        let rows = sqlx::query(
            "SELECT o.title AS option_title, v.value AS value, v.id AS value_id \
             FROM product_option_values v \
             JOIN product_options o ON o.id = v.product_option_id \
             WHERE o.product_id = ?",
        )
        .bind(product_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        option_map_from_rows(&rows)
    }
}

// -------------------------------------------------------------------------
// Transactional variant and link helpers
// -------------------------------------------------------------------------

async fn insert_variants_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: &str,
    variants: &[CreateVariantInput],
    option_map: &HashMap<(String, String), String>,
) -> Result<(), AppError> {
    let mut seen_skus = HashSet::new();
    for variant in variants {
        if let Some(sku) = normalize_optional(variant.sku.clone()) {
            if !seen_skus.insert(sku.to_lowercase()) {
                return Err(AppError::conflict(format!(
                    "duplicate variant sku in input: {sku}"
                )));
            }
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
    if sku != existing.sku {
        if let Some(sku) = sku.as_deref() {
            validate_length(sku, "variant sku", MAX_SKU_LENGTH)?;
            ensure_sku_available_tx(tx, sku, Some(variant_id)).await?;
        }
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

async fn replace_variants_tx(
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

// -------------------------------------------------------------------------
// Link helpers
// -------------------------------------------------------------------------

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

fn option_map_from_rows(rows: &[SqliteRow]) -> Result<HashMap<(String, String), String>, AppError> {
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

async fn sync_search_index_tx(
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

// -------------------------------------------------------------------------
// Row mappers and validation
// -------------------------------------------------------------------------

fn column<'r, T>(row: &'r SqliteRow, name: &str) -> Result<T, AppError>
where
    T: sqlx::Decode<'r, Sqlite> + sqlx::Type<Sqlite>,
{
    row.try_get(name).map_err(database_error)
}

fn metadata_column(row: &SqliteRow, name: &str) -> Result<Metadata, AppError> {
    let value: String = column(row, name)?;
    Ok(Metadata::from_json_str(&value))
}

fn product_from_row(row: &SqliteRow) -> Result<Product, AppError> {
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

fn variant_from_row(row: &SqliteRow) -> Result<ProductVariant, AppError> {
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

fn option_from_row(row: &SqliteRow) -> Result<ProductOption, AppError> {
    Ok(ProductOption {
        id: column(row, "id")?,
        product_id: column(row, "product_id")?,
        title: column(row, "title")?,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
        values: Vec::new(),
    })
}

fn option_value_from_row(row: &SqliteRow) -> Result<ProductOptionValue, AppError> {
    Ok(ProductOptionValue {
        id: column(row, "id")?,
        option_id: column(row, "product_option_id")?,
        value: column(row, "value")?,
        rank: column(row, "rank")?,
        metadata: metadata_column(row, "metadata")?,
    })
}

fn image_from_row(row: &SqliteRow) -> Result<ProductImage, AppError> {
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

fn category_from_row(row: &SqliteRow) -> Result<Category, AppError> {
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

fn collection_from_row(row: &SqliteRow) -> Result<Collection, AppError> {
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

fn tag_from_row(row: &SqliteRow) -> Result<Tag, AppError> {
    Ok(Tag {
        id: column(row, "id")?,
        value: column(row, "value")?,
        metadata: metadata_column(row, "metadata")?,
        created_at: column(row, "created_at")?,
    })
}

fn validate_title(value: &str, field: &str) -> Result<String, AppError> {
    validate_length(value, field, MAX_TITLE_LENGTH)?;
    Ok(value.trim().to_owned())
}

fn resolve_handle(value: Option<&str>, title: &str, field: &str) -> Result<String, AppError> {
    let handle = match value {
        Some(value) => value.trim().to_owned(),
        None => slugify(title),
    };
    validate_handle(&handle, field)?;
    Ok(handle)
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn validate_weight(weight: Option<f64>) -> Result<(), AppError> {
    if let Some(weight) = weight {
        if !weight.is_finite() || weight < 0.0 {
            return Err(AppError::validation(
                "weight must be a finite, non-negative number",
            ));
        }
    }
    Ok(())
}

fn validate_dimensions(dimensions: Dimensions) -> Result<(), AppError> {
    for (value, field) in [
        (dimensions.length, "length"),
        (dimensions.height, "height"),
        (dimensions.width, "width"),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || value < 0.0 {
                return Err(AppError::validation(format!(
                    "{field} must be a finite, non-negative number"
                )));
            }
        }
    }
    Ok(())
}

fn bool_to_int(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

fn checked_pagination(pagination: Pagination) -> Result<Pagination, AppError> {
    Pagination::new(Some(pagination.limit), Some(pagination.offset))
}

fn sql_markers(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

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
