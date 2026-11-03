//! Public storefront theme route.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::routing::get;
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_module_theme::ThemeService;

use super::StoreDb;
use crate::dto::StoreThemeDto;
use crate::state::AppContext;

/// Builds the public theme routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/theme", get(get_active_theme))
}

async fn get_active_theme(StoreDb(db): StoreDb) -> ApiResult<Json<StoreThemeDto>> {
    let theme = ThemeService::new(db)
        .active()
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("theme", "active")))?;
    Ok(Json(StoreThemeDto::from(theme)))
}
