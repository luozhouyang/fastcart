//! Storefront extension manifest, assets and HTML injection.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::get;
use bytes::Bytes;
use fastcart_http::ApiError;
use fastcart_kernel::{AppError, StoreContext};
use fastcart_plugin::inject::{InjectionConfig, StreamInjector};
use futures::StreamExt;
use serde_json::{Value, json};

use super::StoreDb;
use crate::state::AppContext;

const RUNTIME_JS: &str = include_str!("../../assets/extensions-runtime.js");

/// Builds the storefront asset/manifest routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/_fastcart/extensions/runtime.js", get(runtime_js))
        .route("/_fastcart/extensions/manifest.json", get(manifest))
        .route(
            "/_fastcart/extensions/{plugin_id}/{digest}/{*path}",
            get(asset),
        )
}

async fn runtime_js() -> Response {
    js_response(RUNTIME_JS)
}

fn js_response(body: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    // The runtime is not digest-scoped, so a short cache keeps CSRF and boot
    // fixes from waiting an hour behind a cached copy.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}

async fn manifest(
    State(context): State<Arc<AppContext>>,
    Extension(store): Extension<StoreContext>,
    StoreDb(db): StoreDb,
) -> Result<axum::Json<Value>, ApiError> {
    let items = storefront_items(&context, &db, "").await?;
    Ok(axum::Json(json!({
        "v": 1,
        "store": store.store_id.as_str(),
        "items": items,
    })))
}

async fn asset(
    State(context): State<Arc<AppContext>>,
    Extension(_store): Extension<StoreContext>,
    StoreDb(db): StoreDb,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let plugin_id = params.get("plugin_id").cloned().unwrap_or_default();
    let digest = params.get("digest").cloned().unwrap_or_default();
    let path = params.get("path").cloned().unwrap_or_default();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::new(AppError::not_found("plugin asset", path)));
    }
    let installation: Option<(String, i64)> =
        sqlx::query_as("SELECT digest, enabled FROM plugin_installations WHERE plugin_id = ?")
            .bind(&plugin_id)
            .fetch_optional(db.pool())
            .await
            .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((installed, enabled)) = installation else {
        return Err(ApiError::new(AppError::not_found("plugin", plugin_id)));
    };
    if enabled == 0 || installed != digest {
        return Err(ApiError::new(AppError::not_found("plugin asset", path)));
    }
    let bytes =
        context
            .plugin_artifacts
            .read_file(&digest, &path)
            .map_err(|error| match error {
                fastcart_plugin::PluginError::NotFound(_) => {
                    ApiError::new(AppError::not_found("plugin asset", path.clone()))
                }
                fastcart_plugin::PluginError::UnsafePath(_) => ApiError::new(AppError::Forbidden),
                other => ApiError::new(other.into()),
            })?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(&path)),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

async fn storefront_items(
    context: &AppContext,
    db: &fastcart_infrastructure::StoreDatabase,
    path: &str,
) -> Result<Vec<Value>, ApiError> {
    let catalog = super::plugin_catalog::load(context, db)
        .await
        .map_err(ApiError::new)?;
    let mut items = Vec::new();
    for plugin in &catalog.plugins {
        let prefix = format!(
            "/_fastcart/extensions/{}/{}/",
            plugin.plugin_id, plugin.digest
        );
        for extension in &plugin.manifest.extensions.storefront {
            if !matches_path(extension, path) {
                continue;
            }
            items.push(json!({
                "id": plugin.plugin_id,
                "digest": plugin.digest,
                "kind": extension.kind,
                "placement": extension.placement,
                "entry": format!("{prefix}{}", extension.entry.trim_start_matches('/')),
                "css": extension.css.as_ref().map(|css| {
                    format!("{prefix}{}", css.trim_start_matches('/'))
                }),
                "height_hint": extension.height_hint,
            }));
        }
    }
    Ok(items)
}

fn matches_path(extension: &fastcart_plugin::manifest::StorefrontExtension, path: &str) -> bool {
    let path = if path.is_empty() { "/" } else { path };
    if extension
        .exclude_paths
        .iter()
        .any(|pattern| pattern_matches(pattern, path))
    {
        return false;
    }
    if extension.pages.is_empty() {
        return pattern_matches("*", path);
    }
    extension
        .pages
        .iter()
        .any(|pattern| pattern_matches(pattern, path))
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix);
    }
    path == pattern
}

/// Builds the tag configuration for one storefront response.
pub async fn injection_config(
    context: &Arc<AppContext>,
    db: &fastcart_infrastructure::StoreDatabase,
    store_id: &str,
    path: &str,
) -> Result<Option<InjectionConfig>, ApiError> {
    let items = storefront_items(context, db, path).await?;
    if items.is_empty() {
        return Ok(None);
    }
    let mut head = String::new();
    for item in &items {
        if let Some(css) = item["css"].as_str() {
            head.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"{css}\" media=\"all\" />"
            ));
        }
    }
    let manifest = json!({
        "v": 1,
        "store": store_id,
        "items": items,
    });
    let encoded = serde_json::to_string(&manifest)
        .unwrap_or_else(|_| "{\"v\":1,\"items\":[]}".to_owned())
        .replace("</", "<\\/");
    let body = format!(
        "<script>window.__FASTCART_EXTENSIONS__ = {encoded};</script>\
         <script type=\"module\" src=\"/_fastcart/extensions/runtime.js\" async></script>"
    );
    Ok(Some(InjectionConfig { head, body }))
}

/// Applies storefront injection to an HTML response as a stream.
pub async fn inject_response(
    context: &Arc<AppContext>,
    store_id: &str,
    path: &str,
    response: Response,
) -> Response {
    if response.status() != StatusCode::OK {
        return response;
    }
    let is_html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
    if !is_html
        || response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return response;
    }

    let database = match context
        .databases
        .database(&fastcart_kernel::StoreId::from_string(store_id.to_owned()))
        .await
    {
        Ok(database) => database,
        Err(_) => return response,
    };
    let config = match injection_config(context, &database, store_id, path).await {
        Ok(Some(config)) => config,
        _ => return response,
    };
    let (mut parts, body) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, inject_stream(body, config))
}

fn inject_stream(body: Body, config: InjectionConfig) -> Body {
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let mut input = body.into_data_stream();
        let mut injector = StreamInjector::new(config);
        while let Some(chunk) = input.next().await {
            match chunk {
                Ok(bytes) => {
                    for part in injector.push(&bytes) {
                        if sender.send(Ok(Bytes::from(part))).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = sender
                        .send(Err(std::io::Error::other(error.to_string())))
                        .await;
                    return;
                }
            }
        }
        for part in injector.finish() {
            if sender.send(Ok(Bytes::from(part))).await.is_err() {
                return;
            }
        }
    });
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Body::from_stream(stream)
}

/// Returns true for paths owned by the extension runtime.
#[must_use]
pub fn is_runtime_path(path: &str) -> bool {
    path.starts_with("/_fastcart/extensions/")
}
