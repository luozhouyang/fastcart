//! Static client assets from the active universal SSR artifact.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use fastcart_http::ApiError;

use crate::routes::preview::{
    PREVIEW_ASSET_COOKIE, configured_secret, is_revoked, verify_preview_token_unbound,
};
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
    let Some(secret) = configured_secret(&context) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(token) = preview_asset_token(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(payload) = verify_preview_token_unbound(secret, token, fastcart_kernel::now_timestamp())
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if payload.store_id != store.id.as_str() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let database = match context.databases.database(&store.id).await {
        Ok(database) => database,
        Err(error) => return ApiError::new(error.into()).into_response(),
    };
    match is_revoked(&database, &payload.jti, fastcart_kernel::now_timestamp()).await {
        Ok(false) => {}
        Ok(true) => return StatusCode::FORBIDDEN.into_response(),
        Err(error) => return ApiError::new(error).into_response(),
    }
    let mut response = runtime
        .serve_preview_asset(store.id.as_str(), path, headers)
        .await;
    response
        .headers_mut()
        .append(header::VARY, header::HeaderValue::from_static("Cookie"));
    response
}

fn preview_asset_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == PREVIEW_ASSET_COOKIE).then_some(value)
        })
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
