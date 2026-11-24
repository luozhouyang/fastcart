//! edits admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{OrderEditId, OrderId};
use fastcart_module_order::{OrderEdit, OrderService};
use fastcart_workflows::AfterSalesWorkflow;

use super::StoreDb;
use crate::dto::{CancelAfterSaleRequest, CreateOrderEditRequest};
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    get,
    path = "/orders/{id}/edits",
    tag = "Admin",
    summary = "List order edits",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderEdit>))
)]
pub(super) async fn list_order_edits(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderEdit>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_order_edits(&id).await?))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/edits/{edit_id}",
    tag = "Admin",
    summary = "Get an order edit",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderEdit))
)]
pub(super) async fn get_order_edit(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, edit_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderEdit>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit_id = OrderEditId::from_string(edit_id);
    Ok(Json(
        OrderService::new(db).get_order_edit(&id, &edit_id).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/edits",
    tag = "Admin",
    summary = "Create an order edit",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateOrderEditRequest,
    responses((status = 201, body = OrderEdit))
)]
pub(super) async fn create_order_edit(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateOrderEditRequest>,
) -> ApiResult<(StatusCode, Json<OrderEdit>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_order_edit(&id, map_edit_input(request))
        .await?;
    Ok((StatusCode::CREATED, Json(edit)))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/edits/{edit_id}/confirm",
    tag = "Admin",
    summary = "Confirm an order edit",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = OrderEdit))
)]
pub(super) async fn confirm_order_edit(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, edit_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderEdit>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit_id = OrderEditId::from_string(edit_id);
    // The workflow is the layer that owns the order-edit boundary; the line
    // rewrite and the stock movements are one transaction inside it.
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let confirmation = workflow.confirm_order_edit(&id, &edit_id).await?;
    Ok(Json(confirmation.order_edit))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/payment-summary",
    tag = "Admin",
    summary = "Captured, refunded and outstanding money for an order",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PaymentSummaryResponse))
)]
pub(super) async fn payment_summary(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PaymentSummaryResponse>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let summary = workflow.payment_summary(&id).await?;
    let refund_due = summary.refund_due();
    Ok(Json(PaymentSummaryResponse {
        order_id: summary.order_id.as_str().to_owned(),
        currency_code: summary.currency_code,
        total: summary.total,
        captured_total: summary.captured_total,
        refunded_total: summary.refunded_total,
        net_paid: summary.net_paid,
        outstanding: summary.outstanding,
        refund_due,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/edits/{edit_id}/collect-difference",
    tag = "Admin",
    summary = "Make the price difference a confirmed edit left payable",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CollectDifferenceRequest,
    responses((status = 200, body = DifferenceCollectionResponse))
)]
pub(super) async fn collect_order_edit_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, edit_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CollectDifferenceRequest>,
) -> ApiResult<Json<DifferenceCollectionResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit_id = OrderEditId::from_string(edit_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow
        .collect_order_edit_difference(&id, &edit_id, &request.provider)
        .await?;
    Ok(Json(DifferenceCollectionResponse {
        order: result.order,
        payment_collection: result.collection,
        payment_session: result.session,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/edits/{edit_id}/refund-difference",
    tag = "Admin",
    summary = "Refund the price difference a confirmed edit owes the customer",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = DifferenceRefundResponse))
)]
pub(super) async fn refund_order_edit_difference(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, edit_id)): Path<(String, String)>,
) -> ApiResult<Json<DifferenceRefundResponse>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit_id = OrderEditId::from_string(edit_id);
    let workflow = AfterSalesWorkflow::new(db.clone(), payment_service(&context, &actor, &db)?)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let result = workflow.refund_order_edit_difference(&id, &edit_id).await?;
    Ok(Json(DifferenceRefundResponse {
        order: result.order,
        order_refund: result.order_refund,
        provider_refund: result.provider_refund,
    }))
}

#[utoipa::path(
    post,
    path = "/orders/{id}/edits/{edit_id}/cancel",
    tag = "Admin",
    summary = "Cancel an order edit",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CancelAfterSaleRequest,
    responses((status = 200, body = OrderEdit))
)]
pub(super) async fn cancel_order_edit(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, edit_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderEdit>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let edit_id = OrderEditId::from_string(edit_id);
    let edit = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .cancel_order_edit_with_reason(&id, &edit_id, request.reason.as_deref())
        .await?;
    Ok(Json(edit))
}
