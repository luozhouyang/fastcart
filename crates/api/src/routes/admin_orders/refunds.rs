//! refunds admin order routes.

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::OrderId;
use fastcart_module_order::{Order, OrderRefund, OrderService};

use super::StoreDb;
use crate::state::AppContext;

/// Builds the routes for this namespace.
use super::*;

#[utoipa::path(
    post,
    path = "/orders/{id}/refunds",
    tag = "Admin",
    summary = "Refund an order payment",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = RefundOrderRequest,
    responses((status = 200, body = Order))
)]
pub(super) async fn create_refund(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<RefundOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let payment_id = request.payment_id.clone();
    let amount = request.amount;
    let reason = request.reason.clone();
    let payments = payment_service(&context, &actor, &db)?;
    let payment = payments.get_payment(payment_id.as_str()).await?;
    if payment.order_id.as_ref() != Some(&id) {
        return Err(ApiError::validation(
            "refund payment does not belong to this order",
        ));
    }
    if payment.currency_code != request.currency {
        return Err(ApiError::validation(
            "refund currency does not match the payment currency",
        ));
    }
    // Move money first; an order record must never claim a refund that the
    // provider did not accept.
    let idempotency_key = request.idempotency_key.clone();
    payments
        .refund_with_idempotency(payment_id.as_str(), amount, reason, idempotency_key)
        .await?;
    let order = OrderService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .add_refund(&id, map_refund_input(request))
        .await?;
    Ok(Json(order))
}

#[utoipa::path(
    get,
    path = "/orders/{id}/refunds",
    tag = "Admin",
    summary = "List order refunds",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<OrderRefund>))
)]
pub(super) async fn list_refunds(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderRefund>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db).get(&id).await?;
    Ok(Json(order.refunds))
}
