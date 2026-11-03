//! Public storefront catalog routes.
//!
//! Every handler operates on the store resolved from the trusted `Host`; the
//! client cannot select a store. Only published products are exposed.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::{ApiQuery, ApiResult, PaginationQuery};
use fastcart_module_catalog::{CatalogService, Product, ProductFilter, ProductStatus};
use fastcart_module_store::StoreService;
use serde::Deserialize;

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the public catalog routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/products", get(list_products))
        .route("/products/{handle}", get(get_product))
        .route("/categories", get(list_categories))
        .route("/collections", get(list_collections))
        .route("/search", get(search))
        .route("/settings/public", get(public_settings))
}

/// Query parameters accepted by the storefront product listing.
#[derive(Clone, Debug, Default, Deserialize)]
struct ProductQuery {
    /// Page size.
    limit: Option<i64>,
    /// Records to skip.
    offset: Option<i64>,
    /// Restrict to products linked to a category.
    category_id: Option<String>,
    /// Restrict to products linked to a collection.
    collection_id: Option<String>,
    /// Restrict to products carrying a tag value.
    tag: Option<String>,
    /// Case-insensitive text matched against product data.
    query: Option<String>,
}

/// Query parameters accepted by storefront search.
#[derive(Clone, Debug, Default, Deserialize)]
struct SearchQuery {
    /// Search terms.
    q: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Records to skip.
    offset: Option<i64>,
}

async fn list_products(
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ProductQuery>,
) -> ApiResult<Json<PageResponse<Product>>> {
    let pagination = PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    }
    .pagination()?;
    let page = CatalogService::new(db)
        .list_products(storefront_product_filter(query), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn get_product(StoreDb(db): StoreDb, Path(handle): Path<String>) -> ApiResult<Json<Product>> {
    let product = CatalogService::new(db)
        .get_published_by_handle(&handle)
        .await?;
    Ok(Json(product))
}

async fn list_categories(
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<fastcart_module_catalog::Category>>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_published_categories(None, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn list_collections(
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<fastcart_module_catalog::Collection>>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_published_collections(pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn search(
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<PageResponse<Product>>> {
    let pagination = PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    }
    .pagination()?;
    let page = CatalogService::new(db)
        .search_products(query.q.as_deref().unwrap_or_default(), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn public_settings(StoreDb(db): StoreDb) -> ApiResult<Json<BTreeMap<String, String>>> {
    let settings = StoreService::new(db).get_all_settings().await?;
    let public = settings
        .into_iter()
        .filter(|(key, _)| is_public_setting_key(key))
        .collect();
    Ok(Json(public))
}

/// Maps storefront filters onto the catalog filter, always restricting the
/// result to published products.
fn storefront_product_filter(query: ProductQuery) -> ProductFilter {
    ProductFilter {
        status: Some(ProductStatus::Active),
        category_id: query.category_id,
        collection_id: query.collection_id,
        tag_value: query.tag,
        query: query.query,
        include_deleted: false,
    }
}

/// Returns true when a setting key may be exposed to storefronts.
fn is_public_setting_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    !["secret", "password", "token", "api_key"]
        .iter()
        .any(|needle| key.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{ProductQuery, is_public_setting_key, storefront_product_filter};
    use fastcart_module_catalog::ProductStatus;

    #[test]
    fn secret_setting_keys_are_hidden() {
        assert!(is_public_setting_key("store_name"));
        assert!(is_public_setting_key("currency_code"));
        assert!(!is_public_setting_key("stripe_secret_key"));
        assert!(!is_public_setting_key("api_key"));
        assert!(!is_public_setting_key("support_token"));
        assert!(!is_public_setting_key("PASSWORD_RESET_TTL"));
    }

    #[test]
    fn storefront_filters_are_published_only() {
        let filter = storefront_product_filter(ProductQuery {
            category_id: Some("cat_1".to_owned()),
            collection_id: Some("col_1".to_owned()),
            tag: Some("sale".to_owned()),
            query: Some("shirt".to_owned()),
            ..ProductQuery::default()
        });
        assert_eq!(filter.status, Some(ProductStatus::Active));
        assert_eq!(filter.category_id.as_deref(), Some("cat_1"));
        assert_eq!(filter.collection_id.as_deref(), Some("col_1"));
        assert_eq!(filter.tag_value.as_deref(), Some("sale"));
        assert_eq!(filter.query.as_deref(), Some("shirt"));
        assert!(!filter.include_deleted);
    }
}
