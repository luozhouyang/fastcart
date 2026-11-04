//! Public storefront catalog routes.
//!
//! Every handler operates on the store resolved from the trusted `Host`; the
//! client cannot select a store. Only published products are exposed and, when
//! a publishable API key is supplied, only products linked to one of the key's
//! sales channels are visible.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path};
use axum::routing::get;
use fastcart_http::{ApiQuery, ApiResult, PaginationQuery};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, now_timestamp};
use fastcart_module_catalog::{CatalogService, Product, ProductFilter, ProductStatus};
use fastcart_module_pricing::{PriceContext, PriceListType, PricingService};
use fastcart_module_store::{RegionService, StoreService};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{PublishableKeyContext, StoreDb};
use crate::dto::{
    CategoryListResponse, CollectionListResponse, ProductListResponse, ProductResponse,
    ProductSearchResponse,
};
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

/// Builds the documented public catalog routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_products))
        .routes(routes!(get_product))
        .routes(routes!(list_categories))
        .routes(routes!(list_collections))
        .routes(routes!(search))
        .routes(routes!(public_settings))
}

/// Query parameters accepted by the storefront product listing.
#[derive(Clone, Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
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
    /// Currency used for calculated prices; takes precedence over `region_id`.
    currency_code: Option<String>,
    /// Region whose currency is used for calculated prices.
    region_id: Option<String>,
    /// Customer group used when selecting prices.
    customer_group_id: Option<String>,
    /// Quantity used when selecting quantity-tiered prices.
    quantity: Option<i64>,
}

/// Query parameters accepted by storefront search.
#[derive(Clone, Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct SearchQuery {
    /// Search terms.
    q: Option<String>,
    /// Page size.
    limit: Option<i64>,
    /// Records to skip.
    offset: Option<i64>,
}

/// A computed price for a storefront product.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CalculatedPrice {
    /// Amount in minor currency units.
    pub amount: i64,
    /// Currency of the calculated price.
    pub currency_code: String,
    /// Winning price list, or `None` for a base price.
    pub price_list_id: Option<String>,
    /// Type of the winning price list, or `None` for a base price.
    pub price_list_type: Option<PriceListType>,
    /// Base amount the calculated price was compared against, when any.
    pub original_amount: Option<i64>,
    /// Evaluation timestamp in Unix seconds.
    pub calculated_at: i64,
}

/// A product enriched with an optional calculated price.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct StoreProduct {
    /// The catalog product, flattened into the response object.
    #[serde(flatten)]
    pub product: Product,
    /// Calculated price, present only when a currency was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculated_price: Option<CalculatedPrice>,
}

#[utoipa::path(
    get,
    path = "/products",
    tag = "Store",
    summary = "List published products",
    params(ProductQuery),
    responses((status = 200, body = ProductListResponse))
)]
async fn list_products(
    StoreDb(db): StoreDb,
    publishable_key: Option<Extension<PublishableKeyContext>>,
    ApiQuery(query): ApiQuery<ProductQuery>,
) -> ApiResult<Json<ProductListResponse>> {
    let pagination = PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    }
    .pagination()?;
    let channel_ids = key_sales_channel_ids(&publishable_key);
    let page = CatalogService::new(db.clone())
        .list_products(storefront_product_filter(&query, channel_ids), pagination)
        .await?;
    let calculated = calculate_product_prices(&db, &page.data, &query).await?;
    let page = page.map(|product| StoreProduct {
        calculated_price: calculated.get(&product.id).cloned(),
        product,
    });
    Ok(Json(ProductListResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/products/{handle}",
    tag = "Store",
    summary = "Get a published product by handle",
    params(ProductQuery),
    responses((status = 200, body = ProductResponse))
)]
async fn get_product(
    StoreDb(db): StoreDb,
    publishable_key: Option<Extension<PublishableKeyContext>>,
    Path(handle): Path<String>,
    ApiQuery(query): ApiQuery<ProductQuery>,
) -> ApiResult<Json<ProductResponse>> {
    let channel_ids = key_sales_channel_ids(&publishable_key);
    let product = CatalogService::new(db.clone())
        .get_published_by_handle_in_channels(&handle, channel_ids)
        .await?;
    let calculated = calculate_product_prices(&db, std::slice::from_ref(&product), &query).await?;
    Ok(Json(ProductResponse {
        product: StoreProduct {
            calculated_price: calculated.get(&product.id).cloned(),
            product,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/categories",
    tag = "Store",
    summary = "List published categories",
    params(PaginationQuery),
    responses((status = 200, body = CategoryListResponse))
)]
async fn list_categories(
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<CategoryListResponse>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_published_categories(None, pagination)
        .await?;
    Ok(Json(CategoryListResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/collections",
    tag = "Store",
    summary = "List published collections",
    params(PaginationQuery),
    responses((status = 200, body = CollectionListResponse))
)]
async fn list_collections(
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<CollectionListResponse>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_published_collections(pagination)
        .await?;
    Ok(Json(CollectionListResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/search",
    tag = "Store",
    summary = "Search published products",
    params(SearchQuery),
    responses((status = 200, body = ProductSearchResponse))
)]
async fn search(
    StoreDb(db): StoreDb,
    publishable_key: Option<Extension<PublishableKeyContext>>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<ProductSearchResponse>> {
    let pagination = PaginationQuery {
        limit: query.limit,
        offset: query.offset,
    }
    .pagination()?;
    let channel_ids = key_sales_channel_ids(&publishable_key);
    let filter = ProductFilter {
        status: Some(ProductStatus::Active),
        sales_channel_ids: (!channel_ids.is_empty()).then(|| channel_ids.to_vec()),
        include_deleted: false,
        ..ProductFilter::default()
    };
    let page = CatalogService::new(db)
        .search_products_with_filter(query.q.as_deref().unwrap_or_default(), filter, pagination)
        .await?;
    Ok(Json(ProductSearchResponse::from_page(page)))
}

/// Returns the public store settings as a raw key/value map.
///
/// Unlike the collection endpoints, settings have no Medusa envelope: the
/// payload is an arbitrary map of public keys to string values, so wrapping it
/// would add a layer without a stable resource name.
#[utoipa::path(
    get,
    path = "/settings/public",
    tag = "Store",
    summary = "Get public store settings",
    responses((status = 200, body = BTreeMap<String, String>))
)]
async fn public_settings(StoreDb(db): StoreDb) -> ApiResult<Json<BTreeMap<String, String>>> {
    let settings = StoreService::new(db).public_settings().await?;
    let public = settings
        .into_iter()
        .filter(|(key, _)| is_public_setting_key(key))
        .collect();
    Ok(Json(public))
}

/// Returns the sales channels attached to an optional publishable key.
fn key_sales_channel_ids(publishable_key: &Option<Extension<PublishableKeyContext>>) -> &[String] {
    publishable_key.as_ref().map_or(&[], |Extension(context)| {
        context.sales_channel_ids.as_slice()
    })
}

/// Maps storefront filters onto the catalog filter, always restricting the
/// result to published products and, when present, the key's sales channels.
fn storefront_product_filter(query: &ProductQuery, channel_ids: &[String]) -> ProductFilter {
    ProductFilter {
        status: Some(ProductStatus::Active),
        category_id: query.category_id.clone(),
        collection_id: query.collection_id.clone(),
        tag_value: query.tag.clone(),
        query: query.query.clone(),
        sales_channel_ids: (!channel_ids.is_empty()).then(|| channel_ids.to_vec()),
        include_deleted: false,
    }
}

/// Resolves the currency requested by `currency_code` or `region_id`.
async fn resolve_currency(
    db: &StoreDatabase,
    query: &ProductQuery,
) -> ApiResult<Option<CurrencyCode>> {
    if let Some(currency_code) = query.currency_code.as_deref() {
        return Ok(Some(
            CurrencyCode::new(currency_code).map_err(AppError::from)?,
        ));
    }
    let Some(region_id) = query.region_id.as_deref() else {
        return Ok(None);
    };
    let region = RegionService::new(db.clone()).get_region(region_id).await?;
    Ok(Some(
        CurrencyCode::new(&region.currency_code).map_err(AppError::from)?,
    ))
}

/// Calculates the lowest selected price for each product, keyed by product id.
async fn calculate_product_prices(
    db: &StoreDatabase,
    products: &[Product],
    query: &ProductQuery,
) -> ApiResult<HashMap<String, CalculatedPrice>> {
    let Some(currency) = resolve_currency(db, query).await? else {
        return Ok(HashMap::new());
    };
    let variant_ids: Vec<String> = products
        .iter()
        .flat_map(|product| product.variants.iter().map(|variant| variant.id.clone()))
        .collect();
    if variant_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let context = PriceContext {
        currency,
        quantity: query.quantity.unwrap_or(1),
        customer_group_id: query.customer_group_id.clone(),
        sales_channel_id: None,
        at: now_timestamp(),
    };
    let selected = PricingService::new(db.clone())
        .calculate_prices(&variant_ids, &context)
        .await?;

    let mut calculated = HashMap::new();
    for product in products {
        let best = product
            .variants
            .iter()
            .filter_map(|variant| selected.get(&variant.id).and_then(Option::as_ref))
            .min_by_key(|price| price.amount);
        if let Some(price) = best {
            calculated.insert(
                product.id.clone(),
                CalculatedPrice {
                    amount: price.amount,
                    currency_code: price.currency.as_str().to_owned(),
                    price_list_id: price.price_list_id.clone(),
                    price_list_type: price.price_list_type,
                    original_amount: price.original_amount,
                    calculated_at: context.at,
                },
            );
        }
    }
    Ok(calculated)
}

/// Returns true when a setting key may be exposed to storefronts.
fn is_public_setting_key(key: &str) -> bool {
    !fastcart_module_store::is_secret_setting_key(key)
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
        let filter = storefront_product_filter(
            &ProductQuery {
                category_id: Some("cat_1".to_owned()),
                collection_id: Some("col_1".to_owned()),
                tag: Some("sale".to_owned()),
                query: Some("shirt".to_owned()),
                ..ProductQuery::default()
            },
            &[],
        );
        assert_eq!(filter.status, Some(ProductStatus::Active));
        assert_eq!(filter.category_id.as_deref(), Some("cat_1"));
        assert_eq!(filter.collection_id.as_deref(), Some("col_1"));
        assert_eq!(filter.tag_value.as_deref(), Some("sale"));
        assert_eq!(filter.query.as_deref(), Some("shirt"));
        assert_eq!(filter.sales_channel_ids, None);
        assert!(!filter.include_deleted);
    }

    #[test]
    fn storefront_filters_forward_sales_channels() {
        let filter = storefront_product_filter(
            &ProductQuery::default(),
            &["sc_web".to_owned(), "sc_pos".to_owned()],
        );
        assert_eq!(
            filter.sales_channel_ids,
            Some(vec!["sc_web".to_owned(), "sc_pos".to_owned()])
        );
    }
}
