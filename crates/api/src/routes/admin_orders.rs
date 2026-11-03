//! Admin order HTTP routes.
//!
//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{CustomerId, OrderId, PaymentId};
use fastcart_module_order::{
    FulfillmentInput, Order, OrderFilter, OrderRefund, OrderService, OrderStatus,
    OrderStatusHistory, PaymentStatus, RefundInput,
};
use serde::Deserialize;

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/orders", get(list_orders))
        .route("/orders/{id}", get(get_order))
        .route(
            "/orders/by-number/{display_number}",
            get(get_order_by_number),
        )
        .route("/orders/{id}/confirm", post(confirm_order))
        .route("/orders/{id}/process", post(process_order))
        .route("/orders/{id}/cancel", post(cancel_order))
        .route("/orders/{id}/fulfill", post(fulfill_order))
        .route("/orders/{id}/history", get(order_history))
        .route(
            "/orders/{id}/refunds",
            get(list_refunds).post(create_refund),
        )
}

// -------------------------------------------------------------------------
// Requests
// -------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
struct OrderListQuery {
    status: Option<OrderStatus>,
    payment_status: Option<PaymentStatus>,
    customer_id: Option<CustomerId>,
    email: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct CancelOrderRequest {
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct FulfillOrderRequest {
    #[serde(default)]
    tracking_numbers: Vec<String>,
    shipped_at: Option<i64>,
    #[serde(default)]
    partial: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RefundOrderRequest {
    payment_id: PaymentId,
    amount: i64,
    currency: String,
    reason: Option<String>,
}

// -------------------------------------------------------------------------
// Pure request mapping
// -------------------------------------------------------------------------

fn map_order_filter(query: OrderListQuery) -> OrderFilter {
    OrderFilter {
        status: query.status,
        payment_status: query.payment_status,
        customer_id: query.customer_id,
        email: query.email.filter(|email| !email.trim().is_empty()),
        from: None,
        to: None,
    }
}

fn map_fulfillment_input(request: FulfillOrderRequest) -> FulfillmentInput {
    FulfillmentInput {
        tracking_numbers: request.tracking_numbers,
        shipped_at: request.shipped_at,
        partial: request.partial,
    }
}

fn map_refund_input(request: RefundOrderRequest) -> RefundInput {
    RefundInput {
        payment_id: request.payment_id,
        amount: request.amount,
        currency: request.currency,
        reason: request.reason,
    }
}

// -------------------------------------------------------------------------
// Queries
// -------------------------------------------------------------------------

async fn list_orders(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<OrderListQuery>,
) -> ApiResult<Json<PageResponse<Order>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = OrderService::new(db)
        .list(map_order_filter(query), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn get_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).get(&id).await?))
}

async fn get_order_by_number(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(display_number): Path<i64>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    Ok(Json(
        OrderService::new(db)
            .get_by_display_number(display_number)
            .await?,
    ))
}

// -------------------------------------------------------------------------
// Lifecycle transitions
// -------------------------------------------------------------------------

async fn confirm_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).confirm(&id).await?))
}

async fn process_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).start_processing(&id).await?))
}

async fn cancel_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CancelOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.cancel")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db)
        .cancel(&id, request.reason.as_deref())
        .await?;
    Ok(Json(order))
}

async fn fulfill_order(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<FulfillOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db)
        .fulfill(&id, map_fulfillment_input(request))
        .await?;
    Ok(Json(order))
}

async fn order_history(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderStatusHistory>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_history(&id).await?))
}

// -------------------------------------------------------------------------
// Refunds
// -------------------------------------------------------------------------

async fn create_refund(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<RefundOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db)
        .add_refund(&id, map_refund_input(request))
        .await?;
    Ok(Json(order))
}

async fn list_refunds(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_order_filters_and_drops_blank_email() {
        let customer_id = CustomerId::from_string("cust_1");
        let filter = map_order_filter(OrderListQuery {
            status: Some(OrderStatus::Confirmed),
            payment_status: Some(PaymentStatus::Captured),
            customer_id: Some(customer_id.clone()),
            email: Some("buyer@example.com".to_owned()),
        });

        assert_eq!(filter.status, Some(OrderStatus::Confirmed));
        assert_eq!(filter.payment_status, Some(PaymentStatus::Captured));
        assert_eq!(filter.customer_id, Some(customer_id));
        assert_eq!(filter.email.as_deref(), Some("buyer@example.com"));

        let blank = map_order_filter(OrderListQuery {
            status: None,
            payment_status: None,
            customer_id: None,
            email: Some("   ".to_owned()),
        });
        assert!(blank.email.is_none());
    }

    #[test]
    fn maps_fulfillment_input() {
        let input = map_fulfillment_input(FulfillOrderRequest {
            tracking_numbers: vec!["TRACK-1".to_owned()],
            shipped_at: Some(1_700_000_000),
            partial: true,
        });

        assert_eq!(input.tracking_numbers, vec!["TRACK-1".to_owned()]);
        assert_eq!(input.shipped_at, Some(1_700_000_000));
        assert!(input.partial);
    }

    #[test]
    fn maps_refund_input() {
        let payment_id = PaymentId::from_string("pay_1");
        let input = map_refund_input(RefundOrderRequest {
            payment_id: payment_id.clone(),
            amount: 500,
            currency: "USD".to_owned(),
            reason: Some("damaged".to_owned()),
        });

        assert_eq!(input.payment_id, payment_id);
        assert_eq!(input.amount, 500);
        assert_eq!(input.currency, "USD");
        assert_eq!(input.reason.as_deref(), Some("damaged"));
    }
}
