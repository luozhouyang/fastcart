//! Axum presentation layer.
//!
//! Phase 0 exposes health endpoints and reserves the API namespaces defined by
//! the architecture. Route handlers stay thin: they translate HTTP into
//! application service calls and must not contain business rules.

use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

/// Shared state for the HTTP router.
#[derive(Clone, Debug)]
pub struct ApiState {
    /// Service name reported by health endpoints.
    pub service_name: &'static str,
    /// Service version reported by health endpoints.
    pub version: &'static str,
}

impl Default for ApiState {
    fn default() -> Self {
        Self {
            service_name: "fastcart",
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

/// Health response payload.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

/// Builds the application router.
pub fn build_router(state: ApiState) -> Router {
    let reserved = Router::<ApiState>::new().fallback(not_implemented);

    Router::new()
        .route("/health", get(health))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .nest("/api/v1/platform", reserved.clone())
        .nest("/api/v1/admin", reserved.clone())
        .nest("/api/v1/store", reserved.clone())
        .nest("/api/v1/webhooks", reserved.clone())
        .nest("/internal/storefront", reserved)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(health_response(&state))
}

async fn liveness(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(health_response(&state))
}

async fn readiness(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(health_response(&state))
}

fn health_response(state: &ApiState) -> HealthResponse {
    HealthResponse {
        status: "ok",
        service: state.service_name,
        version: state.version,
    }
}

async fn not_implemented(OriginalUri(uri): OriginalUri) -> (StatusCode, Json<serde_json::Value>) {
    let body = serde_json::json!({
        "error": {
            "code": "NOT_IMPLEMENTED",
            "message": format!("route {} is not implemented yet", uri.path()),
            "request_id": null
        }
    });
    (StatusCode::NOT_IMPLEMENTED, Json(body))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::{ApiState, build_router};

    #[tokio::test]
    async fn health_endpoint_is_ok() {
        let app = build_router(ApiState::default());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn reserved_namespaces_are_explicit() {
        let app = build_router(ApiState::default());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/products")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
