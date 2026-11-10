//! Internal plugin runtime endpoints.
//!
//! The workerd loader fetches module graphs from this namespace. Requests are
//! authenticated with a deployment-scoped bearer token and are never reachable
//! from a browser.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;
use fastcart_http::ApiError;
use fastcart_kernel::{AppError, StoreId};
use fastcart_plugin::{PluginError, PluginService, RuntimeBundle};
use serde::Deserialize;

use crate::state::AppContext;

/// Shared state for the runtime bearer check.
#[derive(Clone)]
pub struct PluginRuntimeAuthState {
    /// Expected bearer token.
    pub token: String,
}

/// Builds the internal runtime router.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/bundles/{plugin_id}", get(get_bundle))
}

/// Requires the configured bearer token.
pub async fn auth_middleware(
    State(state): State<PluginRuntimeAuthState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let provided = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if provided.is_empty() || provided != state.token {
        return Err(ApiError::new(AppError::Forbidden));
    }
    Ok(next.run(request).await)
}

/// Bundle query parameters.
#[derive(Clone, Debug, Deserialize)]
pub struct BundleQuery {
    /// Artifact digest to serve.
    pub digest: String,
}

async fn get_bundle(
    State(context): State<Arc<AppContext>>,
    Path((store_id, plugin_id)): Path<(String, String)>,
    Query(query): Query<BundleQuery>,
) -> Result<Json<RuntimeBundle>, ApiError> {
    let store_id = StoreId::from_string(store_id);
    let database = context
        .databases
        .database(&store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    let service = PluginService::new(database, context.plugin_artifacts.clone());
    let bundle = service
        .bundle(&plugin_id, &query.digest)
        .await
        .map_err(|error| match error {
            PluginError::NotFound(_) | PluginError::Disabled(_) => {
                ApiError::new(AppError::not_found("plugin bundle", plugin_id))
            }
            other => ApiError::new(other.into()),
        })?;
    Ok(Json(bundle))
}
