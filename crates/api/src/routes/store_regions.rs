//! Public storefront region routes.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::{ApiResult, PaginationQuery};
use fastcart_module_store::{Region, RegionService};

use super::StoreDb;
use crate::dto::{PageInfo, PageResponse};
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
) -> ApiResult<Json<PageResponse<Region>>> {
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = RegionService::new(db).list_regions(pagination).await?;
    Ok(Json(PageResponse {
        data: page.data,
        pagination: PageInfo {
            limit: page.pagination.limit,
            offset: page.pagination.offset,
            total: page.count,
        },
    }))
}

async fn get_region(StoreDb(db): StoreDb, Path(id): Path<String>) -> ApiResult<Json<Region>> {
    Ok(Json(RegionService::new(db).get_region(&id).await?))
}
