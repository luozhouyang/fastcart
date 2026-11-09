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
    serve_path(context, path, headers).await
}

/// Serves an asset from the store's preview artifact.
pub async fn serve_preview(
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
    runtime
        .serve_preview_asset(store.id.as_str(), path, headers)
        .await
}

/// SvelteKit adapter compatibility: its immutable assets live at `/_app/*`.
pub async fn serve_app(
    State(context): State<Arc<AppContext>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    serve_path(context, format!("_app/{path}"), headers).await
}

async fn serve_path(context: Arc<AppContext>, path: String, headers: HeaderMap) -> Response {
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
