//! Storefront SSR fallback: resolve the store and active theme, then render the
//! request through the configured runtime.
//!
//! This handler is installed as the router fallback so unmatched paths on a
//! storefront host are rendered by the theme bundle. API, health, metrics and
//! internal namespaces are explicitly excluded.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::{
    CONNECTION, CONTENT_LENGTH, HOST, HeaderName, HeaderValue, TE, TRAILER, TRANSFER_ENCODING,
    UPGRADE,
};
use axum::response::{IntoResponse, Response};
use fastcart_http::{ApiError, RequestId};
use fastcart_kernel::{AppError, CurrencyCode, StoreContext, now_timestamp};
use fastcart_module_content::{ContentService, Page, PageStatus};
use fastcart_module_theme::ThemeService;
use fastcart_runtime_api::{
    PageContext, RuntimeBody, RuntimeError, RuntimeMode, RuntimeRequest, RuntimeResponse,
    RuntimeTheme,
};
use fastcart_storefront::{PageBody, ThemeEditorManifest};
use fastcart_storefront_data::{DataMode, DataResolutionContext};
use http_body_util::BodyExt;
use serde_json::json;

use crate::state::AppContext;

/// Renders unmatched storefront requests through the configured runtime.
pub async fn render(State(context): State<Arc<AppContext>>, request: Request) -> Response {
    let path = request.uri().path();
    if is_reserved_path(path) {
        return ApiError::not_found("route").into_response();
    }
    let Some(runtime) = context.storefront_runtime.clone() else {
        return ApiError::not_found("storefront route").into_response();
    };
    let Some(host) = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    else {
        return ApiError::new(AppError::validation("missing Host header")).into_response();
    };
    let store = match context.resolver.resolve_active(&host).await {
        Ok(store) => store,
        Err(_) => return ApiError::not_found("store").into_response(),
    };
    let database = match context.databases.database(&store.id).await {
        Ok(database) => database,
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };
    let currency = match CurrencyCode::new(store.currency_code.clone()) {
        Ok(currency) => currency,
        Err(error) => return ApiError::new(AppError::from(error)).into_response(),
    };
    let theme = match ThemeService::new(database.clone()).active().await {
        Ok(Some(theme)) => theme,
        Ok(None) => return ApiError::not_found("active theme").into_response(),
        Err(error) => return ApiError::new(error).into_response(),
    };
    let editor = theme
        .editor_manifest
        .as_ref()
        .and_then(|value| match ThemeEditorManifest::parse(value) {
            Ok(manifest) => Some(manifest),
            Err(error) => {
                tracing::warn!(error = %error, "stored theme editor manifest is invalid");
                None
            }
        });
    let content = ContentService::with_editor_manifest(database.clone(), editor.clone());
    let page = load_storefront_page(&content, path).await;
    let parsed_page = page.as_ref().and_then(|page| {
        let body = match PageBody::parse(&page.body) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(page_id = %page.id, error = %error, "stored page body is invalid");
                return None;
            }
        };
        if let Some(editor) = editor.as_ref()
            && let Err(error) = body.validate_with_editor(editor)
        {
            tracing::warn!(page_id = %page.id, error = %error, "page does not match the active theme");
            return None;
        }
        Some((page.clone(), body))
    });
    let bindings = match (parsed_page.as_ref(), editor.as_ref()) {
        (Some((_, body)), Some(editor)) => body.collect_bindings(editor).unwrap_or_default(),
        _ => Vec::new(),
    };
    let data = if bindings.is_empty() {
        None
    } else {
        let resolution = DataResolutionContext {
            currency: currency.clone(),
            sales_channel_id: None,
            customer_group_id: None,
            mode: DataMode::Production,
            now: now_timestamp(),
        };
        match context
            .storefront_data
            .resolve(&database, &bindings, &resolution)
            .await
        {
            Ok(bag) if !bag.is_empty() => {
                Some(serde_json::to_value(bag).unwrap_or(serde_json::Value::Null))
            }
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(error = %error, "storefront data resolution failed");
                None
            }
        }
    };
    let page_context = parsed_page.as_ref().map(|(page, body)| PageContext {
        id: page.id.as_str().to_owned(),
        handle: page.handle.clone(),
        status: page.status.as_str().to_owned(),
        body: serde_json::to_value(body).unwrap_or(serde_json::Value::Null),
    });

    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_default();
    let method = request.method().as_str().to_owned();
    let url = request
        .uri()
        .path_and_query()
        .map_or_else(|| path.to_owned(), |value| value.as_str().to_owned());
    let headers = request
        .headers()
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name.as_str()) && name.as_str() != HOST)
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();
    let (_, body) = request.into_parts();
    let body = match body.collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return ApiError::new(AppError::validation(format!(
                "could not read request body: {error}"
            )))
            .into_response();
        }
    };
    let runtime_request = RuntimeRequest {
        request_id: request_id.clone(),
        store: StoreContext {
            store_id: store.id.clone(),
            domain: host,
            locale: store.locale.clone(),
            currency,
            timezone: store.timezone.clone(),
        },
        method,
        url,
        headers,
        body: (!body.is_empty()).then_some(body),
        theme: Some(RuntimeTheme {
            id: theme.manifest.id.clone(),
            version: theme.manifest.version.clone(),
            runtime: theme.manifest.runtime.clone(),
            bundle_path: theme.bundle_path.clone(),
            entry: theme.manifest.entry.clone(),
            settings: theme.settings.clone(),
        }),
        page: page_context,
        data,
        capabilities: BTreeMap::new(),
        mode: RuntimeMode::Production,
        timeout_ms: context.config.storefront_timeout_ms.max(1),
    };

    match runtime.render(runtime_request).await {
        Ok(response) => runtime_response(response),
        Err(RuntimeError::Timeout(timeout_ms)) => runtime_error(
            StatusCode::GATEWAY_TIMEOUT,
            "RUNTIME_TIMEOUT",
            &format!("storefront rendering timed out after {timeout_ms} ms"),
            &request_id,
        ),
        Err(error) => {
            tracing::warn!(error = %error, "storefront runtime failed");
            runtime_error(
                StatusCode::BAD_GATEWAY,
                "RUNTIME_ERROR",
                "storefront rendering failed",
                &request_id,
            )
        }
    }
}

async fn load_storefront_page(content: &ContentService, path: &str) -> Option<Page> {
    let handle = path.trim_matches('/');
    match content
        .get_page_by_handle(if handle.is_empty() { "home" } else { handle })
        .await
    {
        Ok(page) if is_published_page(&page) => return Some(page),
        Ok(_) => {}
        Err(error) if error.is_not_found() => {}
        Err(error) => {
            tracing::warn!(error = %error, "storefront page lookup failed");
        }
    }
    // A home page may use any handle; fall back to the page type when the
    // conventional `home` handle is not present.
    if handle.is_empty() {
        let pagination = fastcart_kernel::Pagination {
            limit: 100,
            offset: 0,
        };
        if let Ok(page) = content
            .list_pages(Some(PageStatus::Published), false, pagination)
            .await
        {
            return page
                .data
                .into_iter()
                .find(|page| page.page_type == "home" && is_published_page(page));
        }
    }
    None
}

fn is_published_page(page: &Page) -> bool {
    page.status == PageStatus::Published && page.deleted_at.is_none()
}

fn runtime_response(response: RuntimeResponse) -> Response {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in response.headers {
        if is_hop_by_hop(&name) || name.eq_ignore_ascii_case(CONTENT_LENGTH.as_str()) {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&value) else {
            continue;
        };
        builder = builder.header(name, value);
    }
    let body = match response.body {
        RuntimeBody::Full(bytes) => Body::from(bytes),
        RuntimeBody::Stream(stream) => Body::from_stream(stream),
    };
    builder.body(body).unwrap_or_else(|error| {
        runtime_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RUNTIME_RESPONSE",
            &format!("invalid storefront response: {error}"),
            "",
        )
    })
}

fn runtime_error(
    status: StatusCode,
    code: &'static str,
    message: &str,
    request_id: &str,
) -> Response {
    let request_id = (!request_id.is_empty()).then(|| request_id.to_owned());
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message,
                "request_id": request_id,
            }
        })),
    )
        .into_response()
}

fn is_reserved_path(path: &str) -> bool {
    path == "/metrics"
        || path.starts_with("/api/")
        || path == "/api"
        || path.starts_with("/internal/")
        || path.starts_with("/health")
}

fn is_hop_by_hop(name: &str) -> bool {
    [
        CONNECTION.as_str(),
        "keep-alive",
        TE.as_str(),
        TRAILER.as_str(),
        TRANSFER_ENCODING.as_str(),
        UPGRADE.as_str(),
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use super::is_reserved_path;

    #[test]
    fn api_and_health_paths_are_reserved() {
        assert!(is_reserved_path("/api/v1/store/products"));
        assert!(is_reserved_path("/api"));
        assert!(is_reserved_path("/internal/storefront/products"));
        assert!(is_reserved_path("/health/ready"));
        assert!(is_reserved_path("/metrics"));
        assert!(!is_reserved_path("/"));
        assert!(!is_reserved_path("/products/hoodie"));
        assert!(!is_reserved_path("/robots.txt"));
    }
}
