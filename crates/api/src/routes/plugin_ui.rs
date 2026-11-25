//! Admin plugin UI asset serving.
//!
//! HTML entries require a short-lived mount token; assets are digest-scoped and
//! immutable. The iframe is sandboxed without `allow-same-origin`, so this
//! origin never shares cookies or storage with the admin shell.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use fastcart_http::ApiError;
use fastcart_kernel::{AppError, StoreId, now_timestamp};
use fastcart_plugin::ContextClaims;
use serde::Deserialize;

use crate::state::AppContext;

/// Content policy for plugin HTML entries.
///
/// The iframe has an opaque origin, so `connect-src` and `form-action` are the
/// remaining exfiltration paths. Images are limited to same-origin assets and
/// data URLs; no https origin is allowed to receive a request.
const PLUGIN_UI_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; font-src 'self'; connect-src 'none'; form-action 'none'; \
     frame-ancestors 'self'; base-uri 'none'";

/// Builds the plugin UI asset routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route(
        "/_fastcart/plugin-ui/{store_id}/{plugin_id}/{digest}/{*path}",
        get(serve),
    )
}

#[derive(Clone, Debug, Deserialize)]
pub struct MountQuery {
    #[serde(default)]
    pub t: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    State(context): State<Arc<AppContext>>,
    Path(params): Path<std::collections::HashMap<String, String>>,
    Query(query): Query<MountQuery>,
) -> Result<Response, ApiError> {
    let store_id = params.get("store_id").cloned().unwrap_or_default();
    let plugin_id = params.get("plugin_id").cloned().unwrap_or_default();
    let digest = params.get("digest").cloned().unwrap_or_default();
    let path = params.get("path").cloned().unwrap_or_default();

    if store_id.is_empty()
        || plugin_id.is_empty()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::new(AppError::not_found("plugin asset", path)));
    }

    let database = context
        .databases
        .database(&StoreId::from_string(store_id.clone()))
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    let installation: Option<(String, i64)> =
        sqlx::query_as("SELECT digest, enabled FROM plugin_installations WHERE plugin_id = ?")
            .bind(&plugin_id)
            .fetch_optional(database.pool())
            .await
            .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((installed_digest, enabled)) = installation else {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    };
    if enabled == 0 || installed_digest != digest {
        return Err(ApiError::new(AppError::not_found("plugin asset", path)));
    }

    let is_html = path.ends_with(".html") || path.ends_with(".htm");
    if is_html {
        let token = query
            .t
            .as_deref()
            .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
        let unsigned = fastcart_plugin::TokenSigner::inspect(token)
            .map_err(|_| ApiError::new(AppError::Unauthorized))?;
        let claims = context
            .plugin_tokens
            .derive(&unsigned.store, &unsigned.sub, &unsigned.digest)
            .verify(token, now_timestamp())
            .map_err(|_| ApiError::new(AppError::Unauthorized))?;
        if claims.store != store_id
            || claims.sub != plugin_id
            || claims.digest != digest
            || !claims.scopes.iter().any(|scope| scope == "ui.mount")
        {
            return Err(ApiError::new(AppError::Forbidden));
        }
    }

    let bytes =
        super::storefront_extensions::read_plugin_file(context.as_ref(), &digest, &path).await?;

    let content_type = content_type(&path);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if is_html {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(PLUGIN_UI_CSP),
        );
    } else {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    let mut response = Response::new(axum::body::Body::from(bytes));
    *response.headers_mut() = headers;
    *response.status_mut() = StatusCode::OK;
    Ok(response)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}

/// Builds a mount token for one plugin UI entry.
#[must_use]
pub fn mount_token(
    context: &AppContext,
    store_id: &str,
    plugin_id: &str,
    digest: &str,
    actor_id: &str,
) -> Option<String> {
    let mut claims = ContextClaims::new(
        plugin_id,
        store_id,
        digest,
        vec!["ui.mount".to_owned()],
        fastcart_plugin::TokenActor::Admin {
            id: actor_id.to_owned(),
            permissions: Vec::new(),
        },
        now_timestamp(),
    );
    claims.exp = now_timestamp() + 3_600;
    context
        .plugin_tokens
        .derive(store_id, plugin_id, digest)
        .mint(&claims)
        .ok()
}

#[allow(dead_code)]
fn _into_response(response: Response) -> Response {
    response.into_response()
}

#[cfg(test)]
mod tests {
    use super::PLUGIN_UI_CSP;

    #[test]
    fn plugin_ui_csp_blocks_network_and_forms() {
        assert!(PLUGIN_UI_CSP.contains("connect-src 'none'"));
        assert!(PLUGIN_UI_CSP.contains("form-action 'none'"));
        assert!(PLUGIN_UI_CSP.contains("img-src 'self' data:"));
        assert!(!PLUGIN_UI_CSP.contains("img-src 'self' data: https:"));
    }
}
