//! Static client assets from the active universal SSR artifact.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::state::AppContext;

/// Serves a client asset, or 404 when no universal runtime is active.
pub async fn serve(State(context): State<Arc<AppContext>>, Path(path): Path<String>) -> Response {
    let Some(runtime) = context.universal_storefront.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    runtime.serve_asset(path).await
}
