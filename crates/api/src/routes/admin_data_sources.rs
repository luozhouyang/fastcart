//! Admin data-source picker routes.
//!
//! These endpoints back the theme editor's reference pickers. They only list
//! candidate records; binding storage and live resolution happen elsewhere.
//! Permissions follow the underlying namespace: catalog sources require
//! `products.read` and content sources require `themes.read`.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiResult};
use fastcart_kernel::{ActorContext, AppError, Pagination};
use fastcart_module_catalog::{CatalogService, ProductFilter, ProductSort, ProductStatus};
use fastcart_module_content::ContentService;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/data-sources/{source}", get(list_data_source))
}

/// Builds the documented admin data-source routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new().routes(routes!(list_data_source))
}

/// One selectable record returned to an editor picker.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct DataPickerItem {
    /// Stable record identifier stored in the binding.
    pub id: String,
    /// URL-safe handle, when the record has one.
    pub handle: String,
    /// Human-readable label.
    pub label: String,
}

/// A page of picker records.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct DataPickerResponse {
    /// Matching records for this page.
    pub items: Vec<DataPickerItem>,
    /// Offset of the next page, or `None` when this is the last page.
    pub next: Option<i64>,
}

/// Query parameters shared by every data-source picker.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct DataSourceQuery {
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    collection_id: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    /// Restrict product queries to active products. Defaults to `true`.
    #[serde(default)]
    published: Option<bool>,
    /// Product ordering: title_asc, title_desc, created_at_asc or created_at_desc.
    #[serde(default)]
    sort: Option<String>,
}

/// Lists candidate records for one data source.
#[utoipa::path(
    get,
    path = "/data-sources/{source}",
    tag = "Admin",
    summary = "List data-source records for the editor picker",
    security(("adminSession" = []), ("apiToken" = [])),
    params(DataSourceQuery),
    responses((status = 200, body = DataPickerResponse))
)]
async fn list_data_source(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(source): Path<String>,
    ApiQuery(query): ApiQuery<DataSourceQuery>,
) -> ApiResult<Json<DataPickerResponse>> {
    let pagination = Pagination::new(query.limit, query.offset).map_err(ApiError::new)?;

    match source.as_str() {
        "collection" => {
            authorize(&actor, "products.read")?;
            let page = CatalogService::new(db)
                .search_collections(query.search.as_deref(), pagination)
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|collection| DataPickerItem {
                    id: collection.id,
                    handle: collection.handle,
                    label: collection.title,
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        "product" | "product_list" | "search" => {
            authorize(&actor, "products.read")?;
            let published = query.published.unwrap_or(true);
            let sort = query
                .sort
                .as_deref()
                .map(|value| {
                    ProductSort::parse(value).ok_or_else(|| {
                        ApiError::validation(format!("invalid product sort: {value}"))
                    })
                })
                .transpose()?;
            let filter = ProductFilter {
                status: published.then_some(ProductStatus::Active),
                collection_id: query.collection_id,
                tag_value: query.tag,
                query: query.search,
                include_deleted: false,
                ..ProductFilter::default()
            };
            let page = CatalogService::new(db)
                .list_products_sorted(filter, sort, pagination)
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|product| DataPickerItem {
                    id: product.id,
                    handle: product.handle,
                    label: product.title,
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        "page" => {
            authorize(&actor, "themes.read")?;
            let page = ContentService::new(db)
                .search_pages(query.search.as_deref(), pagination)
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|page| DataPickerItem {
                    id: page.id.as_str().to_owned(),
                    handle: page.handle,
                    label: page.title,
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        "navigation" => {
            authorize(&actor, "themes.read")?;
            let page = ContentService::new(db)
                .search_navigations(query.search.as_deref(), pagination)
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|navigation| DataPickerItem {
                    id: navigation.id.as_str().to_owned(),
                    handle: navigation.handle,
                    label: navigation.name,
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        "asset" => {
            authorize(&actor, "themes.read")?;
            let page = fastcart_module_asset::AssetService::new(db)
                .list_assets(
                    fastcart_module_asset::AssetFilter {
                        search: query.search,
                    },
                    pagination,
                )
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|asset| DataPickerItem {
                    id: asset.id,
                    handle: String::new(),
                    label: asset.alt.unwrap_or(asset.filename),
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        "inquiry_form" => {
            authorize(&actor, "themes.read")?;
            let page = fastcart_module_inquiry::InquiryFormService::new(db)
                .list_published_forms(query.search.as_deref(), pagination)
                .await?;
            let items = page
                .data
                .into_iter()
                .map(|form| DataPickerItem {
                    id: form.id.as_str().to_owned(),
                    handle: form.handle,
                    label: form.name,
                })
                .collect();
            Ok(Json(picker_response(page.count, pagination, items)))
        }
        _ => Err(ApiError::new(AppError::not_found("data source", &source))),
    }
}

/// Enforces a permission at the handler boundary.
fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

fn picker_response(
    count: i64,
    pagination: Pagination,
    items: Vec<DataPickerItem>,
) -> DataPickerResponse {
    let next = pagination.offset.saturating_add(pagination.limit);
    DataPickerResponse {
        items,
        next: (next < count).then_some(next),
    }
}
