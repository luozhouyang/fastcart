//! Public storefront region routes.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::{ApiResult, PaginationQuery};
use fastcart_module_store::RegionService;

use super::StoreDb;
use crate::dto::{RegionListResponse, RegionResponse};
use crate::state::AppContext;

/// Builds the public region routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/regions", get(list_regions))
        .route("/regions/{id}", get(get_region))
}

async fn list_regions(
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<RegionListResponse>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = RegionService::new(db).list_regions(pagination).await?;
    Ok(Json(RegionListResponse::from_page(page)))
}

async fn get_region(
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<RegionResponse>> {
    let region = RegionService::new(db).get_region(&id).await?;
    Ok(Json(RegionResponse { region }))
}
