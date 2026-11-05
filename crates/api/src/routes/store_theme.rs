//! Public storefront theme route.

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_module_theme::ThemeService;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{StoreThemeDto, StoreThemeResponse};
use crate::state::AppContext;

/// Maximum size of one client asset served from a theme bundle.
///
/// Client bundles are JavaScript, CSS and images; anything larger is a sign of
/// a packaging mistake and would otherwise be buffered per request.
const MAX_THEME_ASSET_BYTES: u64 = 16 * 1024 * 1024;

/// Builds the public theme routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/theme", get(get_active_theme))
        .route("/theme-assets/{*path}", get(get_theme_asset))
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

/// Serves one client asset from the active theme bundle.
async fn get_theme_asset(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    Path(path): Path<String>,
) -> ApiResult<Response> {
    let theme = ThemeService::new(db)
        .active()
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("theme", "active")))?;
    let client_dir = theme
        .assets_manifest
        .as_ref()
        .and_then(|manifest| manifest.get("entries"))
        .and_then(|entries| entries.get("client"))
        .and_then(|client| {
            client
                .as_str()
                .or_else(|| client.get("path").and_then(serde_json::Value::as_str))
        })
        .unwrap_or("dist/client");
    if !safe_relative_path(client_dir) || !safe_relative_path(&path) {
        return Err(ApiError::new(AppError::not_found("theme asset", &path)));
    }

    let root = tokio::fs::canonicalize(&context.config.themes_root)
        .await
        .map_err(|_| ApiError::new(AppError::not_found("theme asset", &path)))?;
    let bundle = tokio::fs::canonicalize(root.join(&theme.bundle_path))
        .await
        .map_err(|_| ApiError::new(AppError::not_found("theme asset", &path)))?;
    if !bundle.starts_with(&root) {
        return Err(ApiError::new(AppError::not_found("theme asset", &path)));
    }
    let client_root = tokio::fs::canonicalize(bundle.join(client_dir))
        .await
        .map_err(|_| ApiError::new(AppError::not_found("theme asset", &path)))?;
    if !client_root.starts_with(&bundle) {
        return Err(ApiError::new(AppError::not_found("theme asset", &path)));
    }
    let target = tokio::fs::canonicalize(client_root.join(&path))
        .await
        .map_err(|_| ApiError::new(AppError::not_found("theme asset", &path)))?;
    if !target.starts_with(&client_root) || !target.is_file() {
        return Err(ApiError::new(AppError::not_found("theme asset", &path)));
    }
    let metadata = tokio::fs::metadata(&target)
        .await
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;
    if metadata.len() > MAX_THEME_ASSET_BYTES {
        tracing::warn!(
            asset = %target.display(),
            size = metadata.len(),
            "theme asset exceeds the serving limit"
        );
        return Err(ApiError::new(AppError::not_found("theme asset", &path)));
    }
    let bytes = tokio::fs::read(&target)
        .await
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;
    let len = bytes.len();
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(&target)),
    );
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    headers.insert(
        axum::http::header::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && !path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

fn content_type_for(path: &FsPath) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
