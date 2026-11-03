//! Health and readiness endpoints.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::{Value, json};

use crate::state::AppContext;

/// Builds health routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/health", get(health))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
}

async fn health(State(context): State<Arc<AppContext>>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": context.config.service_name,
        "version": context.config.version,
    }))
}

async fn liveness() -> Json<Value> {
    Json(json!({"status": "ok"}))
}

async fn readiness(State(context): State<Arc<AppContext>>) -> (StatusCode, Json<Value>) {
    match context.platform.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "service": context.config.service_name,
                "version": context.config.version,
            })),
        ),
        Err(error) => {
            tracing::error!(error = %error, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status": "degraded"})),
            )
        }
    }
}
