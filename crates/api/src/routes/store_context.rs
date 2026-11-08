//! Public store context for theme and storefront SDK clients.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Extension;
use axum::routing::get;
use fastcart_http::ApiResult;
use fastcart_kernel::StoreContext;
use fastcart_module_store::RegionService;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the storefront context routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/context", get(store_context))
}

/// Builds the documented storefront context routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new().routes(routes!(store_context))
}

/// Store identity and defaults a storefront client needs before its first call.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct StoreContextDto {
    /// Store identifier.
    pub store_id: String,
    /// Hostname that resolved to this store.
    pub domain: String,
    /// Store locale, for example `en-US`.
    pub locale: String,
    /// Display currency in ISO 4217 form.
    pub currency_code: String,
    /// Default region of the store, when one is configured.
    pub default_region_id: Option<String>,
    /// IANA time zone identifier.
    pub timezone: String,
}

#[utoipa::path(
    get,
    path = "/context",
    tag = "Store",
    summary = "Get public store context",
    responses((status = 200, body = StoreContextDto))
)]
async fn store_context(
    Extension(store): Extension<StoreContext>,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<StoreContextDto>> {
    let default_region_id = RegionService::new(db)
        .default_region()
        .await?
        .map(|region| region.id);
    Ok(Json(StoreContextDto {
        store_id: store.store_id.to_string(),
        domain: store.domain,
        locale: store.locale,
        currency_code: store.currency.to_string(),
        default_region_id,
        timezone: store.timezone,
    }))
}
