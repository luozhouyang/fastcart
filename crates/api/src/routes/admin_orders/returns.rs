//! returns admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{OrderId, ReturnId};
use fastcart_module_order::{OrderReturn, OrderService};
use fastcart_workflows::AfterSalesWorkflow;

use super::StoreDb;
use crate::dto::{
    CancelAfterSaleRequest, CreateReturnRequest, ReceiveReturnRequest, WriteOffReturnRequest,
};
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    get,
    path = "/orders/{id}/returns",
    tag = "Admin",
    summary = "List order returns",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderReturn>))
)]
pub(super) async fn list_returns(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderReturn>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_returns(&id).await?))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/returns/{return_id}",
    tag = "Admin",
    summary = "Get an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderReturn))
)]
pub(super) async fn get_return(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderReturn>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    Ok(Json(
        OrderService::new(db).get_return(&id, &return_id).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns",
    tag = "Admin",
    summary = "Create an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateReturnRequest,
    responses((status = 201, body = OrderReturn))
)]
pub(super) async fn create_return(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateReturnRequest>,
) -> ApiResult<(StatusCode, Json<OrderReturn>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order_return = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_return(&id, map_return_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(order_return)))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns/{return_id}/request",
    tag = "Admin",
    summary = "Request an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderReturn))
)]
pub(super) async fn request_return(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderReturn>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    let order_return = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .request_return(&id, &return_id)
        .await?;
    Ok(Json(order_return))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns/{return_id}/receive",
    tag = "Admin",
    summary = "Receive an order return and restock received items",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ReceiveReturnRequest,
    responses((status = 200, body = OrderReturn))
)]
pub(super) async fn receive_return(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<ReceiveReturnRequest>,
) -> ApiResult<Json<OrderReturn>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .receive_return(&id, &return_id, map_receive_input(request))
        .await?;
    let order_return = result.order_return;
    Ok(Json(order_return))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns/{return_id}/refund",
    tag = "Admin",
    summary = "Refund an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = ReturnRefundResponse))
)]
pub(super) async fn refund_return(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<ReturnRefundResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .refund_return(&id, &return_id, request.reason)
        .await?;
    Ok(Json(ReturnRefundResponse {
        order: result.order,
        order_return: result.order_return,
        order_refund: result.order_refund,
        provider_refund: result.provider_refund,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns/{return_id}/write-off",
    tag = "Admin",
    summary = "Write off an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = WriteOffReturnRequest,
    responses((status = 200, body = OrderReturn))
)]
pub(super) async fn write_off_return(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<WriteOffReturnRequest>,
) -> ApiResult<Json<OrderReturn>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    let order_return = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .write_off_return(&id, &return_id, map_write_off_input(request))
        .await?;
    Ok(Json(order_return))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/returns/{return_id}/cancel",
    tag = "Admin",
    summary = "Cancel an order return",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = OrderReturn))
)]
pub(super) async fn cancel_return(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, return_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderReturn>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let return_id = ReturnId::from_string(return_id);
    let order_return = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .cancel_return_with_reason(&id, &return_id, request.reason.as_deref())
        .await?;
    Ok(Json(order_return))
}
