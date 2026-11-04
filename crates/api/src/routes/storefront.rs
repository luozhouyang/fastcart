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
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::{
    CONNECTION, CONTENT_LENGTH, HOST, HeaderName, HeaderValue, TE, TRAILER, TRANSFER_ENCODING,
    UPGRADE,
};
use axum::response::{IntoResponse, Response};
use fastcart_http::{ApiError, CART_ID_COOKIE, CUSTOMER_SESSION_COOKIE, RequestId, parse_cookies};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CartId, CurrencyCode, StoreContext, StoreId, now_timestamp};
use fastcart_module_cart::CartService;
use fastcart_module_content::{ContentService, Page, PageStatus};
use fastcart_module_customer::CustomerService;
use fastcart_module_theme::ThemeService;
use fastcart_runtime_api::{
    PageContext, RuntimeBody, RuntimeError, RuntimeMode, RuntimeRequest, RuntimeResponse,
    RuntimeTheme,
};
use fastcart_storefront::{PageBody, StorefrontError, ThemeEditorManifest};
use fastcart_storefront_data::{DataMode, DataResolutionContext};
use http_body_util::BodyExt;
use serde_json::json;

use crate::state::{AppContext, CompiledPage};

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
    let editor = resolve_active_editor(&context, &store.id, &theme).await;
    let content = ContentService::new(database.clone());
    let page = load_storefront_page(&content, path).await;
    let compiled = match page {
        Some(page) => {
            let page_key = format!(
                "{}:{}:{}:{}",
                store.id.as_str(),
                page.id.as_str(),
                page.revision,
                editor.key
            );
            match context.storefront_page_cache.get(&page_key).await {
                Some(compiled) => Some(compiled),
                None => match compile_page(&page, editor.manifest.as_deref()) {
                    Ok(compiled) => {
                        let compiled = Arc::new(compiled);
                        context
                            .storefront_page_cache
                            .insert(page_key, compiled.clone())
                            .await;
                        Some(compiled)
                    }
                    Err(error) => {
                        tracing::warn!(
                            page_id = %page.id,
                            error = %error,
                            "stored page body is invalid"
                        );
                        None
                    }
                },
            }
        }
        None => None,
    };
    let mut data = BTreeMap::new();
    data.insert(
        fastcart_storefront::page::RESERVED_CONTEXT_KEY.to_owned(),
        build_context_data(
            &database,
            request.headers(),
            &store,
            &host,
            &currency,
            &theme,
            DataMode::Production,
        )
        .await,
    );
    if let Some(compiled) = compiled.as_ref()
        && !compiled.bindings.is_empty()
    {
        let resolution = DataResolutionContext {
            currency: currency.clone(),
            sales_channel_id: None,
            customer_group_id: None,
            mode: DataMode::Production,
            now: now_timestamp(),
        };
        match context
            .storefront_data
            .resolve(&database, &compiled.bindings, &resolution)
            .await
        {
            Ok(bag) => data.extend(bag),
            Err(error) => {
                context.metrics.record_storefront_data_failure();
                tracing::warn!(error = %error, "storefront data resolution failed");
            }
        }
    }
    let data = Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null));
    let page_context = compiled.as_ref().map(|compiled| compiled.page.clone());

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
    // Storefront paths are flat: nested paths have no page handle and are
    // treated as an ordinary miss rather than being scanned for a match.
    let result = if handle.is_empty() {
        content.get_home_page().await
    } else {
        content.get_published_page_by_handle(handle).await.map(Some)
    };
    match result {
        Ok(Some(page)) if is_published_page(&page) => Some(page),
        Ok(_) => None,
        Err(error) if error.is_not_found() => None,
        Err(error) => {
            tracing::warn!(error = %error, "storefront page lookup failed");
            None
        }
    }
}

/// A parsed active theme editor manifest and the cache key it was stored under.
///
/// The manifest is `None` when the stored value is absent or invalid, so an
/// invalid manifest is not re-parsed on every render.
pub(crate) struct ActiveEditor {
    /// Cache key derived from the store, theme identifier and revision.
    pub(crate) key: String,
    /// Parsed editor manifest, when the theme provides a valid one.
    pub(crate) manifest: Option<Arc<ThemeEditorManifest>>,
}

/// Loads and caches the active theme's editor manifest.
pub(crate) async fn resolve_active_editor(
    context: &AppContext,
    store_id: &StoreId,
    theme: &fastcart_module_theme::Theme,
) -> ActiveEditor {
    // The internal theme record id is unique per registration; `manifest.id`
    // can stay the same across re-registrations while the stored manifest
    // changes, so it must not be the cache identity.
    let key = format!(
        "{}:{}:{}",
        store_id.as_str(),
        theme.id.as_str(),
        theme.updated_at
    );
    let manifest = context
        .storefront_editor_cache
        .get_with(key.clone(), async {
            theme.editor_manifest.as_ref().and_then(|value| {
                match ThemeEditorManifest::parse(value) {
                    Ok(manifest) => Some(Arc::new(manifest)),
                    Err(error) => {
                        tracing::warn!(error = %error, "stored theme editor manifest is invalid");
                        None
                    }
                }
            })
        })
        .await;
    ActiveEditor { key, manifest }
}

/// Parses and validates a page, then extracts its bindings in the same pass.
pub(crate) fn compile_page(
    page: &Page,
    editor: Option<&ThemeEditorManifest>,
) -> Result<CompiledPage, StorefrontError> {
    let body = PageBody::parse(&page.body)?;
    let bindings = match editor {
        Some(editor) => {
            body.validate_with_editor(editor)?;
            body.collect_bindings_after_validation(editor)?
        }
        None => Vec::new(),
    };
    Ok(CompiledPage {
        page: PageContext {
            id: page.id.as_str().to_owned(),
            handle: page.handle.clone(),
            status: page.status.as_str().to_owned(),
            body: serde_json::to_value(&body).unwrap_or(serde_json::Value::Null),
        },
        bindings,
    })
}

/// Builds the reserved `context` entry injected into every data bag.
///
/// The authenticated customer and their active cart are resolved from
/// store-scoped cookies. Lookup failures degrade to anonymous/null context
/// rather than failing the render: a theme must always receive a stable shape.
pub(crate) async fn build_context_data(
    db: &StoreDatabase,
    headers: &HeaderMap,
    store: &fastcart_platform::Store,
    host: &str,
    currency: &CurrencyCode,
    theme: &fastcart_module_theme::Theme,
    mode: DataMode,
) -> serde_json::Value {
    let cookies = parse_cookies(headers);
    let customer = match cookies.get(CUSTOMER_SESSION_COOKIE) {
        Some(token) => match CustomerService::new(db.clone())
            .session_customer(token)
            .await
        {
            Ok(customer) => customer,
            Err(error) => {
                tracing::warn!(error = %error, "could not resolve customer session context");
                None
            }
        },
        None => None,
    };
    let cart = resolve_context_cart(
        db,
        cookies.get(CART_ID_COOKIE).map(String::as_str),
        customer.as_ref(),
    )
    .await;
    let customer = customer.map(|customer| {
        json!({
            "id": customer.id.as_str(),
            "email": customer.email,
            "first_name": customer.first_name,
            "last_name": customer.last_name,
        })
    });
    json!({
        "store": {
            "id": store.id.as_str(),
            "name": store.name,
            "domain": host,
            "locale": store.locale,
            "currency": currency.as_str(),
            "timezone": store.timezone,
        },
        "theme": {
            "id": theme.manifest.id,
            "version": theme.manifest.version,
            "settings": theme.settings,
        },
        "cart": cart,
        "customer": customer,
        "mode": mode.as_str(),
    })
}

/// Resolves the active cart for the request context.
///
/// An authenticated customer's newest open cart wins. Guests may present a
/// cart id cookie, but only for a guest cart; a customer-scoped cart must never
/// be exposed by guessing an identifier.
async fn resolve_context_cart(
    db: &StoreDatabase,
    cookie: Option<&str>,
    customer: Option<&fastcart_module_customer::Customer>,
) -> serde_json::Value {
    let service = CartService::new(db.clone());
    let mut cart = None;
    if let Some(customer) = customer {
        match service.active_cart_for_customer(&customer.id).await {
            Ok(found) => cart = found,
            Err(error) => {
                tracing::warn!(error = %error, "could not resolve customer cart context");
            }
        }
    }
    if cart.is_none()
        && let Some(id) = cookie
    {
        match service.get_cart(&CartId::from_string(id)).await {
            Ok(found) => {
                let belongs = match (&found.customer_id, customer) {
                    (None, _) => true,
                    (Some(owner), Some(customer)) => owner == &customer.id,
                    (Some(_), None) => false,
                };
                if belongs && found.status == fastcart_module_cart::CartStatus::Open {
                    cart = Some(found);
                }
            }
            Err(error) if error.is_not_found() => {}
            Err(error) => {
                tracing::warn!(error = %error, "could not resolve cart cookie context");
            }
        }
    }
    cart.map_or(serde_json::Value::Null, |cart| {
        json!({
            "id": cart.id.as_str(),
            "currency": cart.currency_code.as_str(),
            "status": cart.status.as_str(),
            "item_count": cart.items.iter().map(|item| item.quantity).sum::<i64>(),
            "subtotal": cart.totals.subtotal,
            "total": cart.totals.total,
        })
    })
}

fn is_published_page(page: &Page) -> bool {
    page.status == PageStatus::Published && page.deleted_at.is_none()
}

pub(crate) fn runtime_response(response: RuntimeResponse) -> Response {
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

pub(crate) fn runtime_error(
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

pub(crate) fn is_hop_by_hop(name: &str) -> bool {
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
