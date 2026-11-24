//! Full-text search and index maintenance.

use super::*;

impl CatalogService {
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
}

impl CatalogService {
    /// Searches all non-deleted products for administration using FTS5.
    pub async fn search_products_admin(
        &self,
        query: &str,
        pagination: Pagination,
    ) -> Result<PageResult<Product>, AppError> {
        self.search_products_with_filter(query, ProductFilter::default(), pagination)
            .await
    }
}

impl CatalogService {
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
}

impl CatalogService {
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
}
