//! Axum presentation layer.
//!
//! Route handlers stay thin: they translate HTTP into application service calls
//! and never contain business rules. Namespaces:
//!
//! - `/api/v1/platform` — installation and store management
//! - `/api/v1/admin` — store-scoped administration
//! - `/api/v1/store` — public storefront operations
//! - `/api/v1/webhooks` — provider callbacks
//! - `/internal/storefront` — legacy trusted SSR-to-commerce boundary
//! - `/internal/ssr/v1/stores/{store_id}` — server-only SSR commerce API
//!
//! Unmatched paths on a storefront host are rendered by the active theme via
//! the configured runtime. API, health, metrics and internal namespaces are
//! excluded from SSR rendering.
//!
//! Middleware order (outermost first): request id → metrics → tracing → body
//! limit → CORS. Route-specific authentication and store resolution are layered
//! per namespace.

pub mod dto;
pub mod openapi;
pub mod routes;
pub mod state;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use tokio::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use state::{ApiConfig, AppContext};

/// Builds the application router.
pub fn build_router(context: Arc<AppContext>) -> Router {
    let mut router = Router::new()
        .merge(routes::health::router())
        .nest("/api/v1/platform", routes::platform::router(&context))
        .nest("/api/v1/admin", routes::admin::router(&context))
        .nest("/api/v1/store", routes::store::router(&context))
        .nest("/api/v1/webhooks", routes::webhooks::router(&context))
        .route("/api/v1/openapi.json", axum::routing::get(openapi::handler))
        // Draft preview is resolved by the Host header exactly like the
        // storefront fallback, but is an explicit route so it never falls
        // through to production rendering.
        .route(
            "/preview",
            axum::routing::get(routes::preview::render),
        )
        .route(
            "/preview/{*path}",
            axum::routing::get(routes::preview::render),
        );

    // The internal surface is only reachable over HTTP when a secret is
    // configured; otherwise it remains an in-process bridge target.
    if let Some(secret) = context.config.internal_fetch_secret.clone() {
        let internal = routes::store::internal_router(&context).layer(
            axum::middleware::from_fn_with_state(secret, routes::internal_fetch_auth_middleware),
        );
        router = router.nest("/internal/storefront", internal);
    }
    if let Some(token) = context.config.internal_ssr_token.clone() {
        let internal = routes::store::internal_ssr_router(&context).layer(
            axum::middleware::from_fn_with_state(token, routes::internal_ssr_auth_middleware),
        );
        router = router.nest("/internal/ssr/v1/stores/{store_id}", internal);
    }

    let router = router
        .fallback(routes::storefront::render)
        .with_state(context.clone());
    let upload = routes::store::upload_router(&context).with_state(context.clone());
    let asset_upload = routes::admin::upload_router(&context).with_state(context.clone());

    // JSON endpoints keep the small limit; attachments and media uploads are
    // merged as separate routers carrying their larger multipart limits.
    let router = with_common_layers(router, &context, context.config.max_body_bytes);
    let upload = with_common_layers(upload, &context, context.config.max_upload_bytes);
    let asset_upload = with_common_layers(
        asset_upload,
        &context,
        context.config.max_asset_upload_bytes,
    );
    router.merge(upload).merge(asset_upload)
}

/// Applies the middleware shared by the main and upload routers.
fn with_common_layers(router: Router, context: &AppContext, body_limit: usize) -> Router {
    router
        .layer(cors_layer(&context.config.allowed_origins))
        .layer(RequestBodyLimitLayer::new(body_limit))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        // Log only the path: preview tokens travel in the query string and
        // must never reach access logs.
        .layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!(
                    "http.request",
                    method = %request.method(),
                    path = %request.uri().path(),
                    version = ?request.version(),
                )
            },
        ))
        .layer(axum::middleware::from_fn_with_state(
            fastcart_http::SecurityHeadersConfig {
                cookie_secure: context.config.cookie_secure,
                preview_frame_ancestors: context.config.preview_frame_ancestors.clone(),
            },
            fastcart_http::security_headers_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            context.client_ip.clone(),
            fastcart_http::client_ip_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            context.metrics.clone(),
            fastcart_http::metrics_middleware,
        ))
        .layer(axum::middleware::from_fn(
            fastcart_http::request_id_middleware,
        ))
}

fn cors_layer(origins: &[String]) -> CorsLayer {
    let allowed_headers = [
        header::CONTENT_TYPE,
        header::AUTHORIZATION,
        header::ACCEPT,
        HeaderName::from_static("x-store-id"),
        HeaderName::from_static("x-csrf-token"),
        HeaderName::from_static("x-publishable-api-key"),
        HeaderName::from_static("x-request-id"),
    ];
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(allowed_headers)
        .allow_credentials(true);
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| origin.parse::<HeaderValue>().ok())
        .collect();
    if parsed.is_empty() {
        layer
    } else {
        layer.allow_origin(parsed)
    }
}
