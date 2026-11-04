//! Public storefront content routes.
//!
//! Pages are only served once published; navigation trees are read-only.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::ApiResult;
use fastcart_module_content::ContentService;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{NavigationResponse, StorePageResponse};
use crate::state::AppContext;

/// Builds the public content routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/pages/{handle}", get(get_page))
        .route("/navigation/{handle}", get(get_navigation))
}

/// Builds the documented public content routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(get_page))
        .routes(routes!(get_navigation))
}

#[utoipa::path(
    get,
    path = "/pages/{handle}",
    tag = "Store",
    summary = "Get a published content page",
    responses((status = 200, body = StorePageResponse))
)]
async fn get_page(
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<Json<StorePageResponse>> {
    let page = ContentService::new(db)
        .get_published_page_by_handle(&handle)
        .await?;
    Ok(Json(StorePageResponse { page }))
}

#[utoipa::path(
    get,
    path = "/navigation/{handle}",
    tag = "Store",
    summary = "Get a navigation tree",
    responses((status = 200, body = NavigationResponse))
)]
async fn get_navigation(
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<Json<NavigationResponse>> {
    let tree = ContentService::new(db).get_navigation_tree(&handle).await?;
    Ok(Json(NavigationResponse { navigation: tree }))
}
