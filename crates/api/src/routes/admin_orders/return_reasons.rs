//! Configurable return reason admin routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read` and `orders.write` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::Metadata;
use fastcart_module_order::{
    CreateReturnReasonInput, OrderService, ReturnReason, ReturnReasonId, UpdateReturnReasonInput,
};
use serde::Deserialize;

use super::StoreDb;
use crate::state::AppContext;

/// Request body for creating a return reason.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateReturnReasonRequest {
    /// Stable machine-readable value, unique per store.
    pub value: String,
    /// Human-readable label snapshotted onto new after-sales records.
    pub label: String,
    /// Optional description.
    pub description: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

impl CreateReturnReasonRequest {
    fn into_input(self) -> CreateReturnReasonInput {
        CreateReturnReasonInput {
            value: self.value,
            label: self.label,
            description: self.description,
            metadata: self.metadata,
        }
    }
}

/// Request body for partially updating a return reason.
///
/// Omitted fields keep their stored value; an explicit `null` description
/// clears it.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateReturnReasonRequest {
    /// Replacement machine-readable value.
    pub value: Option<String>,
    /// Replacement label.
    pub label: Option<String>,
    /// Replacement description; `null` clears it.
    #[serde(default, deserialize_with = "nullable")]
    pub description: Option<Option<String>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

impl UpdateReturnReasonRequest {
    fn into_input(self) -> UpdateReturnReasonInput {
        UpdateReturnReasonInput {
            value: self.value,
            label: self.label,
            description: self.description,
            metadata: self.metadata,
        }
    }
}

/// Distinguishes an explicit `null` from an omitted field.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[utoipa::path(
    get,
    path = "/return-reasons",
    tag = "Admin",
    summary = "List configured return reasons",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<ReturnReason>))
)]
pub(super) async fn list_return_reasons(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<Vec<ReturnReason>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    Ok(Json(OrderService::new(db).list_return_reasons().await?))
}

#[utoipa::path(
    post,
    path = "/return-reasons",
    tag = "Admin",
    summary = "Create a configured return reason",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateReturnReasonRequest,
    responses((status = 201, body = ReturnReason))
)]
pub(super) async fn create_return_reason(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateReturnReasonRequest>,
) -> ApiResult<(StatusCode, Json<ReturnReason>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let reason = OrderService::new(db)
        .create_return_reason(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(reason)))
}

#[utoipa::path(
    get,
    path = "/return-reasons/{id}",
    tag = "Admin",
    summary = "Get a configured return reason",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ReturnReason))
)]
pub(super) async fn get_return_reason(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ReturnReason>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = ReturnReasonId::from_string(id);
    Ok(Json(OrderService::new(db).get_return_reason(&id).await?))
}

#[utoipa::path(
    patch,
    path = "/return-reasons/{id}",
    tag = "Admin",
    summary = "Update a configured return reason",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateReturnReasonRequest,
    responses((status = 200, body = ReturnReason))
)]
pub(super) async fn update_return_reason(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateReturnReasonRequest>,
) -> ApiResult<Json<ReturnReason>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = ReturnReasonId::from_string(id);
    let reason = OrderService::new(db)
        .update_return_reason(&id, request.into_input())
        .await?;
    Ok(Json(reason))
}

#[utoipa::path(
    delete,
    path = "/return-reasons/{id}",
    tag = "Admin",
    summary = "Delete a configured return reason",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn delete_return_reason(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = ReturnReasonId::from_string(id);
    OrderService::new(db).delete_return_reason(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
