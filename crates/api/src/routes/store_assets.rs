//! Public media asset serving.
//!
//! Asset URLs are stable and store-scoped: the `Host` header resolves the
//! store exactly like every other storefront request, so an asset identifier
//! cannot be used to read another store's media.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_module_asset::AssetService;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

const X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
const CONTENT_SECURITY_POLICY: HeaderName = HeaderName::from_static("content-security-policy");
const REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");

/// Builds the public media routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/assets/{id}", get(get_asset))
}

/// Builds the documented public media routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new().routes(routes!(get_asset))
}

#[utoipa::path(
    get,
    path = "/assets/{id}",
    tag = "Store",
    summary = "Fetch a media asset",
    params(("id" = String, Path, description = "Asset identifier")),
    responses((status = 200, description = "Raw asset bytes"), (status = 404))
)]
async fn get_asset(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let asset = AssetService::new(db)
        .get_asset(&id)
        .await
        .map_err(|error| {
            if error.is_not_found() {
                ApiError::not_found("asset")
            } else {
                ApiError::new(error)
            }
        })?;
    let bytes = context
        .storage
        .get(&asset.storage_key)
        .await
        .map_err(|error| {
            ApiError::new(AppError::internal(format!("could not read asset: {error}")))
        })?;

    let len = bytes.len();
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&asset.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    // Asset identifiers are content-addressed by upload and never change, so
    // the response can be cached aggressively. Bytes are only ever uploaded
    // through sniffed media types.
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // Uploaded media is served from the storefront origin; neutralize any
    // script capability even for formats a browser might interpret.
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    Ok(response)
}
