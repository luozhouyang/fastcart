//! Product authoring and lifecycle.

use super::*;

impl CatalogService {
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
        let product_type_id = normalize_optional(input.product_type_id);
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;

        if let Some(product_type_id) = product_type_id.as_deref() {
            ensure_product_type_exists_tx(&mut tx, product_type_id).await?;
        }

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
              width, metadata, product_type_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .bind(&product_type_id)
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
}

impl CatalogService {
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

        let product_type_id = match input.product_type_id {
            Some(value) => normalize_optional(Some(value)),
            None => existing.product_type_id.clone(),
        };

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
        if product_type_id != existing.product_type_id
            && let Some(product_type_id) = product_type_id.as_deref()
        {
            ensure_product_type_exists_tx(&mut tx, product_type_id).await?;
        }

        sqlx::query(
            "UPDATE products SET title = ?, subtitle = ?, description = ?, handle = ?, status = ?, \
             thumbnail = ?, weight = ?, length = ?, height = ?, width = ?, metadata = ?, \
             product_type_id = ?, updated_at = ? WHERE id = ?",
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
        .bind(&product_type_id)
        .bind(now_timestamp())
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_or_database(error, "product handle already exists"))?;

        let option_map = match input.options.as_ref() {
            Some(options) => sync_options_tx(&mut tx, id, options).await?,
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
}

impl CatalogService {
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
        self.append_audit(
            &mut tx,
            "product.deleted",
            "product",
            id,
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl CatalogService {
    /// Publishes a draft or active product.
    pub async fn publish_product(&self, id: &str) -> Result<Product, AppError> {
        let existing = self.get_product(id).await?;
        if !existing.status.can_publish() {
            return Err(AppError::conflict("archived products cannot be published"));
        }
        if existing.status == ProductStatus::Active {
            return Ok(existing);
        }
        self.set_product_status(id, ProductStatus::Active, None)
            .await
    }
}

impl CatalogService {
    /// Archives a product. Archived products are no longer purchasable.
    pub async fn archive_product(&self, id: &str) -> Result<Product, AppError> {
        let existing = self.get_product(id).await?;
        if existing.status == ProductStatus::Archived {
            return Ok(existing);
        }
        self.set_product_status(id, ProductStatus::Archived, Some("product.archived"))
            .await
    }
}

impl CatalogService {
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

        // Replacing the links with the same set is a no-op and must not emit
        // an audit row for a change that did not happen.
        let existing: HashSet<String> = sqlx::query_scalar(
            "SELECT sales_channel_id FROM product_sales_channels WHERE product_id = ?",
        )
        .bind(product_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_error)?
        .into_iter()
        .collect();
        if existing.len() == seen.len() && seen.iter().all(|id| existing.contains(id.as_str())) {
            tx.commit().await.map_err(database_error)?;
            return Ok(());
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
        self.append_audit(
            &mut tx,
            "product.sales_channels_updated",
            "product",
            product_id,
            serde_json::json!({"sales_channel_ids": channel_ids}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl CatalogService {
    async fn set_product_status(
        &self,
        id: &str,
        status: ProductStatus,
        audit_action: Option<&str>,
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
        if let Some(action) = audit_action {
            self.append_audit(&mut tx, action, "product", id, serde_json::Value::Null)
                .await?;
        }
        tx.commit().await.map_err(database_error)?;
        self.get_product(id).await
    }
}
