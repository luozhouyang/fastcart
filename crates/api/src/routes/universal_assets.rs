//! Static client assets from the active universal SSR artifact.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::state::AppContext;

/// Serves a client asset for the store resolved from `Host`.
pub async fn serve(
    State(context): State<Arc<AppContext>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(runtime) = context.universal_storefront.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(store) = context.resolver.resolve_active(host).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    runtime.serve_asset(store.id.as_str(), path, headers).await
}
