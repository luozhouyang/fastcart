//! Public storefront theme route.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::routing::get;
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_module_theme::ThemeService;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{StoreThemeDto, StoreThemeResponse};
use crate::state::AppContext;

/// Builds the public theme routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/theme", get(get_active_theme))
}

/// Builds the documented public theme routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new().routes(routes!(get_active_theme))
}

#[utoipa::path(
    get,
    path = "/theme",
    tag = "Store",
    summary = "Get the active storefront theme",
    responses((status = 200, body = StoreThemeResponse))
)]
async fn get_active_theme(StoreDb(db): StoreDb) -> ApiResult<Json<StoreThemeResponse>> {
    let theme = ThemeService::new(db)
        .active()
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("theme", "active")))?;
    Ok(Json(StoreThemeResponse {
        theme: StoreThemeDto::from(theme),
    }))
}
