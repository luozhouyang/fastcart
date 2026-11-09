//! Server-only activation and rollback endpoints for universal deployments.

use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use fastcart_http::{ApiError, ApiResult};
use fastcart_kernel::AppError;
use fastcart_platform::StoreStatus;
use serde::Deserialize;

use super::InternalSsrScope;
use crate::state::AppContext;

/// Builds the deployment lifecycle router.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/{store_id}/activate", post(activate))
        .route("/{store_id}/rollback", post(rollback))
        .route("/{store_id}/preview/activate", post(activate_preview))
        .route("/{store_id}/preview/deactivate", post(deactivate_preview))
}

#[derive(Debug, Deserialize)]
struct ActivateRequest {
    deployment_id: String,
    artifact_path: String,
}

async fn activate(
    State(context): State<Arc<AppContext>>,
    Path(store_id): Path<String>,
    Extension(scope): Extension<InternalSsrScope>,
    Json(request): Json<ActivateRequest>,
) -> ApiResult<Response> {
    ensure_scope(&scope, &store_id)?;
    ensure_active_store(&context, &store_id).await?;
    let Some(runtime) = context.universal_storefront.clone() else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    runtime
        .activate(&store_id, &request.deployment_id, request.artifact_path)
        .await
        .map_err(|error| ApiError::new(AppError::validation(error)))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn rollback(
    State(context): State<Arc<AppContext>>,
    Path(store_id): Path<String>,
    Extension(scope): Extension<InternalSsrScope>,
) -> ApiResult<Response> {
    ensure_scope(&scope, &store_id)?;
    ensure_active_store(&context, &store_id).await?;
    let Some(runtime) = context.universal_storefront.clone() else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    runtime
        .rollback(&store_id)
        .await
        .map_err(|error| ApiError::new(AppError::conflict(error)))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn activate_preview(
    State(context): State<Arc<AppContext>>,
    Path(store_id): Path<String>,
    Extension(scope): Extension<InternalSsrScope>,
    Json(request): Json<ActivateRequest>,
) -> ApiResult<Response> {
    ensure_scope(&scope, &store_id)?;
    ensure_active_store(&context, &store_id).await?;
    let Some(runtime) = context.universal_storefront.clone() else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    runtime
        .activate_preview(&store_id, &request.deployment_id, request.artifact_path)
        .await
        .map_err(|error| ApiError::new(AppError::validation(error)))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn deactivate_preview(
    State(context): State<Arc<AppContext>>,
    Path(store_id): Path<String>,
    Extension(scope): Extension<InternalSsrScope>,
) -> ApiResult<Response> {
    ensure_scope(&scope, &store_id)?;
    ensure_active_store(&context, &store_id).await?;
    let Some(runtime) = context.universal_storefront.clone() else {
        return Ok(StatusCode::SERVICE_UNAVAILABLE.into_response());
    };
    runtime
        .deactivate_preview(&store_id)
        .await
        .map_err(|error| ApiError::new(AppError::conflict(error)))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn ensure_scope(scope: &InternalSsrScope, store_id: &str) -> ApiResult<()> {
    if scope
        .store_id
        .as_deref()
        .is_some_and(|allowed| allowed != store_id)
    {
        return Err(ApiError::new(AppError::Forbidden));
    }
    Ok(())
}

async fn ensure_active_store(context: &Arc<AppContext>, store_id: &str) -> ApiResult<()> {
    let store_id = fastcart_kernel::StoreId::from_string(store_id.to_owned());
    context
        .platform
        .get_store(&store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?
        .filter(|store| store.status == StoreStatus::Active)
        .ok_or_else(|| ApiError::new(AppError::not_found("store", store_id.as_str())))?;
    Ok(())
}
