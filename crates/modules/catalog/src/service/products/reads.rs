//! Product reads and hydration.

use super::*;

impl CatalogService {
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
}

impl CatalogService {
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
}

impl CatalogService {
    /// Lists products using administrative filters.
    pub async fn list_products(
        &self,
        filter: ProductFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        self.list_products_sorted(filter, None, pagination).await
    }
}

impl CatalogService {
    /// Lists products with an optional storefront ordering.
    pub async fn list_products_sorted(
        &self,
        filter: ProductFilter,
        sort: Option<ProductSort>,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        let pagination = checked_pagination(pagination)?;
        let (where_clause, binds) = product_filter_clause(&filter);
        let order_by = sort.map_or("p.created_at ASC, p.id ASC", ProductSort::order_by);

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
             ORDER BY {order_by} LIMIT ? OFFSET ?"
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
}

impl CatalogService {
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
}

impl CatalogService {
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
}

impl CatalogService {
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
}

impl CatalogService {
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
}

impl CatalogService {
    pub(crate) async fn hydrate_products(&self, products: &mut [Product]) -> Result<(), AppError> {
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
}

impl CatalogService {
    pub(crate) async fn load_variant_option_value_ids(
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

    pub(crate) async fn ensure_product_exists(&self, id: &str) -> Result<(), AppError> {
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

    pub(crate) async fn option_map_from_pool(
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
