//! Axum presentation layer.
//!
//! Route handlers stay thin: they translate HTTP into application service calls
//! and never contain business rules. Namespaces:
//!
//! - `/api/v1/platform` — installation and store management
//! - `/api/v1/admin` — store-scoped administration
//! - `/api/v1/store` — public storefront operations
//! - `/api/v1/webhooks` — provider callbacks
//! - `/internal/storefront` — trusted SSR-to-commerce boundary
//!
//! Middleware order (outermost first): request id → tracing → body limit →
//! CORS. Route-specific authentication and store resolution are layered per
//! namespace.

pub mod dto;
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
    let router = Router::new()
        .merge(routes::health::router())
        .nest("/api/v1/platform", routes::platform::router(&context))
        .nest("/api/v1/admin", routes::admin::router(&context))
        .nest("/api/v1/store", routes::store::router(&context))
        .nest("/api/v1/webhooks", routes::webhooks::router(&context))
        .nest(
            "/internal/storefront",
            routes::store::internal_router(&context),
        )
        .with_state(context.clone());

    router
        .layer(cors_layer(&context.config.allowed_origins))
        .layer(RequestBodyLimitLayer::new(context.config.max_body_bytes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(30),
        ))
        .layer(TraceLayer::new_for_http())
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
