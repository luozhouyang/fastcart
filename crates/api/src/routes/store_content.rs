//! Public storefront content routes.
//!
//! Pages are only served once published; navigation trees are read-only.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::routing::get;
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_module_content::{ContentService, PageStatus};

use super::StoreDb;
use crate::dto::{NavigationResponse, StorePageResponse};
use crate::state::AppContext;

/// Builds the public content routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/pages/{handle}", get(get_page))
        .route("/navigation/{handle}", get(get_navigation))
}

async fn get_page(
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<Json<StorePageResponse>> {
    let page = ContentService::new(db).get_page_by_handle(&handle).await?;
    if !is_public_page_status(page.status) {
        return Err(ApiError::new(AppError::not_found("page", handle)));
    }
    Ok(Json(StorePageResponse { page }))
}

async fn get_navigation(
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<Json<NavigationResponse>> {
    let tree = ContentService::new(db).get_navigation_tree(&handle).await?;
    Ok(Json(NavigationResponse { navigation: tree }))
}

/// Returns true when a page state is visible to storefronts.
fn is_public_page_status(status: PageStatus) -> bool {
    status == PageStatus::Published
}

#[cfg(test)]
mod tests {
    use super::is_public_page_status;
    use fastcart_module_content::PageStatus;

    #[test]
    fn only_published_pages_are_public() {
        assert!(is_public_page_status(PageStatus::Published));
        assert!(!is_public_page_status(PageStatus::Draft));
        assert!(!is_public_page_status(PageStatus::Archived));
    }
}
