//! Health and readiness endpoints.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;

use crate::state::AppContext;

/// Builds health routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/health", get(health))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics))
}

/// Service health summary.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct HealthStatus {
    /// Health state.
    pub status: String,
    /// Service name.
    #[schema(required = false)]
    pub service: String,
    /// Service version.
    #[schema(required = false)]
    pub version: String,
}

/// Liveness result.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct LivenessStatus {
    /// Health state.
    pub status: String,
}

/// Readiness result.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ReadinessStatus {
    /// Health state.
    pub status: String,
    /// Service name, absent when the check fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Service version, absent when the check fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

async fn metrics(State(context): State<Arc<AppContext>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        context.metrics.render(),
    )
}

async fn health(State(context): State<Arc<AppContext>>) -> Json<HealthStatus> {
    Json(HealthStatus {
        status: "ok".to_owned(),
        service: context.config.service_name.clone(),
        version: context.config.version.clone(),
    })
}

async fn liveness() -> Json<LivenessStatus> {
    Json(LivenessStatus {
        status: "ok".to_owned(),
    })
}

async fn readiness(State(context): State<Arc<AppContext>>) -> (StatusCode, Json<ReadinessStatus>) {
    match context.platform.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadinessStatus {
                status: "ok".to_owned(),
                service: Some(context.config.service_name.clone()),
                version: Some(context.config.version.clone()),
            }),
        ),
        Err(error) => {
            tracing::error!(error = %error, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadinessStatus {
                    status: "degraded".to_owned(),
                    service: None,
                    version: None,
                }),
            )
        }
    }
}
