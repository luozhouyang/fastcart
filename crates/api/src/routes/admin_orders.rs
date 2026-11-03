//! Admin order HTTP routes.
//!
//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{
    ClaimId, CustomerId, ExchangeId, LineItemId, OrderEditId, OrderId, PaymentId, ProductId,
    RefundId, ReturnId, VariantId,
};
use fastcart_module_inventory::InventoryService;
use fastcart_module_order::{
    ClaimItemInput, CreateClaimInput, CreateExchangeInput, CreateOrderEditInput, CreateReturnInput,
    ExchangeItemInput, FulfillmentInput, FulfillmentItemInput, Order, OrderClaim, OrderEdit,
    OrderEditItemInput, OrderExchange, OrderFilter, OrderRefund, OrderReturn, OrderService,
    OrderStatus, OrderStatusHistory, PaymentStatus, ReceiveReturnInput, ReceiveReturnItemInput,
    RefundInput, ReturnItemInput, WriteOffReturnInput, WriteOffReturnItemInput,
};
use fastcart_module_payment::PaymentService;
use serde::Deserialize;

use super::{StoreDb, record_store_audit};
use crate::dto::{
    CancelAfterSaleRequest, CreateClaimRequest, CreateExchangeRequest, CreateOrderEditRequest,
    CreateReturnRequest, PageResponse, ReceiveReturnRequest, WriteOffReturnRequest,
};
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
        .route(
            "/orders/{id}/returns",
            get(list_returns).post(create_return),
        )
        .route("/orders/{id}/returns/{return_id}", get(get_return))
        .route(
            "/orders/{id}/returns/{return_id}/request",
            post(request_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/receive",
            post(receive_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/write-off",
            post(write_off_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/cancel",
            post(cancel_return),
        )
        .route("/orders/{id}/claims", get(list_claims).post(create_claim))
        .route("/orders/{id}/claims/{claim_id}", get(get_claim))
        .route("/orders/{id}/claims/{claim_id}/accept", post(accept_claim))
        .route("/orders/{id}/claims/{claim_id}/cancel", post(cancel_claim))
        .route(
            "/orders/{id}/exchanges",
            get(list_exchanges).post(create_exchange),
        )
        .route("/orders/{id}/exchanges/{exchange_id}", get(get_exchange))
        .route(
            "/orders/{id}/exchanges/{exchange_id}/accept",
            post(accept_exchange),
        )
        .route(
            "/orders/{id}/exchanges/{exchange_id}/cancel",
            post(cancel_exchange),
        )
        .route(
            "/orders/{id}/edits",
            get(list_order_edits).post(create_order_edit),
        )
        .route("/orders/{id}/edits/{edit_id}", get(get_order_edit))
        .route(
            "/orders/{id}/edits/{edit_id}/confirm",
            post(confirm_order_edit),
        )
        .route(
            "/orders/{id}/edits/{edit_id}/cancel",
            post(cancel_order_edit),
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
    provider_id: Option<String>,
    #[serde(default)]
    provider_data: serde_json::Value,
    #[serde(default)]
    items: Vec<FulfillmentItemRequest>,
    #[serde(default)]
    tracking_numbers: Vec<String>,
    shipped_at: Option<i64>,
    #[serde(default)]
    partial: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct FulfillmentItemRequest {
    order_item_id: LineItemId,
    quantity: i64,
}

#[derive(Clone, Debug, Deserialize)]
struct RefundOrderRequest {
    payment_id: PaymentId,
    amount: i64,
    currency: String,
    reason: Option<String>,
    idempotency_key: Option<String>,
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
        provider_id: request.provider_id,
        provider_data: request.provider_data,
        items: request
            .items
            .into_iter()
            .map(|item| FulfillmentItemInput {
                order_item_id: item.order_item_id,
                quantity: item.quantity,
            })
            .collect(),
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

fn map_return_input(request: CreateReturnRequest) -> CreateReturnInput {
    CreateReturnInput {
        reason: request.reason,
        refund_id: request.refund_id.map(RefundId::from_string),
        metadata: request.metadata,
        items: request
            .items
            .into_iter()
            .map(|item| ReturnItemInput {
                order_item_id: LineItemId::from_string(item.order_item_id),
                quantity: item.quantity,
                reason: item.reason,
            })
            .collect(),
    }
}

fn map_receive_input(request: ReceiveReturnRequest) -> ReceiveReturnInput {
    ReceiveReturnInput {
        items: request
            .items
            .into_iter()
            .map(|item| ReceiveReturnItemInput {
                order_item_id: LineItemId::from_string(item.order_item_id),
                quantity: item.quantity,
            })
            .collect(),
    }
}

fn map_write_off_input(request: WriteOffReturnRequest) -> WriteOffReturnInput {
    WriteOffReturnInput {
        items: request
            .items
            .into_iter()
            .map(|item| WriteOffReturnItemInput {
                order_item_id: LineItemId::from_string(item.order_item_id),
                quantity: item.quantity,
            })
            .collect(),
    }
}

fn map_claim_input(request: CreateClaimRequest) -> CreateClaimInput {
    CreateClaimInput {
        reason: request.reason,
        metadata: request.metadata,
        items: request
            .items
            .into_iter()
            .map(|item| ClaimItemInput {
                order_item_id: LineItemId::from_string(item.order_item_id),
                quantity: item.quantity,
                item_type: item.item_type,
                replacement_variant_id: item.replacement_variant_id.map(VariantId::from_string),
                reason: item.reason,
            })
            .collect(),
    }
}

fn map_exchange_input(request: CreateExchangeRequest) -> CreateExchangeInput {
    let inbound_items = request
        .inbound_items
        .into_iter()
        .map(|item| ExchangeItemInput {
            order_item_id: LineItemId::from_string(item.order_item_id),
            quantity: item.quantity,
            replacement_variant_id: item.replacement_variant_id.map(VariantId::from_string),
            reason: item.reason,
        })
        .collect();
    let outbound_items = request
        .outbound_items
        .into_iter()
        .map(|item| ExchangeItemInput {
            order_item_id: LineItemId::from_string(item.order_item_id),
            quantity: item.quantity,
            replacement_variant_id: item.replacement_variant_id.map(VariantId::from_string),
            reason: item.reason,
        })
        .collect();
    CreateExchangeInput {
        reason: request.reason,
        metadata: request.metadata,
        inbound_items,
        outbound_items,
    }
}

fn map_edit_input(request: CreateOrderEditRequest) -> CreateOrderEditInput {
    CreateOrderEditInput {
        reason: request.reason,
        metadata: request.metadata,
        items: request
            .items
            .into_iter()
            .map(|item| OrderEditItemInput {
                order_item_id: item.order_item_id.map(LineItemId::from_string),
                action: item.action,
                product_id: item.product_id.map(ProductId::from_string),
                variant_id: item.variant_id.map(VariantId::from_string),
                product_title: item.product_title,
                variant_title: item.variant_title,
                sku: item.sku,
                unit_price: item.unit_price,
                before_quantity: item.before_quantity,
                after_quantity: item.after_quantity,
            })
            .collect(),
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db).confirm(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.confirmed",
        "order",
        order.id.as_str(),
        serde_json::Value::Null,
    )
    .await;
    Ok(Json(order))
}

async fn process_order(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let order = OrderService::new(db).start_processing(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.processing_started",
        "order",
        order.id.as_str(),
        serde_json::Value::Null,
    )
    .await;
    Ok(Json(order))
}

async fn cancel_order(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CancelOrderRequest>,
) -> ApiResult<Json<Order>> {
    actor
        .require_permission("orders.cancel")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let orders = OrderService::new(db.clone());
    let current = orders.get(&id).await?;
    if current.status == OrderStatus::Canceled {
        // Restocking is idempotent; retry it after an interrupted cancellation.
        InventoryService::new(db).restock_order(&id).await?;
        return Ok(Json(current));
    }
    let reason = request.reason;
    // Claim the cancellation first so a concurrent fulfillment cannot ship an
    // order that was already restocked.
    let order = orders.cancel(&id, reason.as_deref()).await?;
    InventoryService::new(db).restock_order(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.canceled",
        "order",
        order.id.as_str(),
        serde_json::json!({"reason": reason}),
    )
    .await;
    Ok(Json(order))
}

async fn fulfill_order(
    State(context): State<Arc<AppContext>>,
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
    record_store_audit(
        &context,
        &actor,
        "order.fulfilled",
        "order",
        order.id.as_str(),
        serde_json::json!({
            "fulfillment_status": order.fulfillment_status.as_str(),
            "status": order.status.as_str(),
        }),
    )
    .await;
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
    let payments = PaymentService::with_registry(db.clone(), context.payment_providers.clone());
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
        .add_refund(&id, map_refund_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.refunded",
        "order",
        order.id.as_str(),
        serde_json::json!({
            "payment_id": payment_id,
            "amount": amount,
            "currency_code": order.currency_code.clone(),
        }),
    )
    .await;
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

// -------------------------------------------------------------------------
// Returns
// -------------------------------------------------------------------------

async fn list_returns(
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

async fn get_return(
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

async fn create_return(
    State(context): State<Arc<AppContext>>,
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
        .create_return(&id, map_return_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.return_created",
        "order_return",
        order_return.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(order_return)))
}

async fn request_return(
    State(context): State<Arc<AppContext>>,
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
        .request_return(&id, &return_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.return_requested",
        "order_return",
        order_return.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok(Json(order_return))
}

async fn receive_return(
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
    let order_return = OrderService::new(db)
        .receive_return(&id, &return_id, map_receive_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.return_received",
        "order_return",
        order_return.id.as_str(),
        serde_json::json!({
            "order_id": id.as_str(),
            "status": order_return.status.as_str(),
        }),
    )
    .await;
    Ok(Json(order_return))
}

async fn write_off_return(
    State(context): State<Arc<AppContext>>,
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
        .write_off_return(&id, &return_id, map_write_off_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.return_written_off",
        "order_return",
        order_return.id.as_str(),
        serde_json::json!({
            "order_id": id.as_str(),
            "status": order_return.status.as_str(),
        }),
    )
    .await;
    Ok(Json(order_return))
}

async fn cancel_return(
    State(context): State<Arc<AppContext>>,
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
    let order_return = OrderService::new(db).cancel_return(&id, &return_id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.return_canceled",
        "order_return",
        order_return.id.as_str(),
        serde_json::json!({"order_id": id.as_str(), "reason": request.reason}),
    )
    .await;
    Ok(Json(order_return))
}

// -------------------------------------------------------------------------
// Claims
// -------------------------------------------------------------------------

async fn list_claims(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderClaim>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_claims(&id).await?))
}

async fn get_claim(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    Ok(Json(OrderService::new(db).get_claim(&id, &claim_id).await?))
}

async fn create_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateClaimRequest>,
) -> ApiResult<(StatusCode, Json<OrderClaim>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim = OrderService::new(db)
        .create_claim(&id, map_claim_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.claim_created",
        "order_claim",
        claim.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(claim)))
}

async fn accept_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let claim = OrderService::new(db).accept_claim(&id, &claim_id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.claim_accepted",
        "order_claim",
        claim.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok(Json(claim))
}

async fn cancel_claim(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, claim_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderClaim>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let claim_id = ClaimId::from_string(claim_id);
    let claim = OrderService::new(db).cancel_claim(&id, &claim_id).await?;
    record_store_audit(
        &context,
        &actor,
        "order.claim_canceled",
        "order_claim",
        claim.id.as_str(),
        serde_json::json!({"order_id": id.as_str(), "reason": request.reason}),
    )
    .await;
    Ok(Json(claim))
}

// -------------------------------------------------------------------------
// Exchanges
// -------------------------------------------------------------------------

async fn list_exchanges(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<OrderExchange>>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    Ok(Json(OrderService::new(db).list_exchanges(&id).await?))
}

async fn get_exchange(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.read")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    Ok(Json(
        OrderService::new(db)
            .get_exchange(&id, &exchange_id)
            .await?,
    ))
}

async fn create_exchange(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateExchangeRequest>,
) -> ApiResult<(StatusCode, Json<OrderExchange>)> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange = OrderService::new(db)
        .create_exchange(&id, map_exchange_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.exchange_created",
        "order_exchange",
        exchange.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(exchange)))
}

async fn accept_exchange(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let exchange = OrderService::new(db)
        .accept_exchange(&id, &exchange_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.exchange_accepted",
        "order_exchange",
        exchange.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok(Json(exchange))
}

async fn cancel_exchange(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, exchange_id)): Path<(String, String)>,
    ApiJson(request): ApiJson<CancelAfterSaleRequest>,
) -> ApiResult<Json<OrderExchange>> {
    actor
        .require_permission("orders.write")
        .map_err(ApiError::new)?;
    let id = OrderId::from_string(id);
    let exchange_id = ExchangeId::from_string(exchange_id);
    let exchange = OrderService::new(db)
        .cancel_exchange(&id, &exchange_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.exchange_canceled",
        "order_exchange",
        exchange.id.as_str(),
        serde_json::json!({"order_id": id.as_str(), "reason": request.reason}),
    )
    .await;
    Ok(Json(exchange))
}

// -------------------------------------------------------------------------
// Order edits
// -------------------------------------------------------------------------

async fn list_order_edits(
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

async fn get_order_edit(
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

async fn create_order_edit(
    State(context): State<Arc<AppContext>>,
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
        .create_order_edit(&id, map_edit_input(request))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.edit_created",
        "order_edit",
        edit.id.as_str(),
        serde_json::json!({"order_id": id.as_str()}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(edit)))
}

async fn confirm_order_edit(
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
    let edit = OrderService::new(db)
        .confirm_order_edit(&id, &edit_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.edit_confirmed",
        "order_edit",
        edit.id.as_str(),
        serde_json::json!({"order_id": id.as_str(), "after_total": edit.after_total}),
    )
    .await;
    Ok(Json(edit))
}

async fn cancel_order_edit(
    State(context): State<Arc<AppContext>>,
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
        .cancel_order_edit(&id, &edit_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "order.edit_canceled",
        "order_edit",
        edit.id.as_str(),
        serde_json::json!({"order_id": id.as_str(), "reason": request.reason}),
    )
    .await;
    Ok(Json(edit))
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
            provider_id: Some("manual".to_owned()),
            provider_data: serde_json::json!({"warehouse": "eu"}),
            items: vec![FulfillmentItemRequest {
                order_item_id: LineItemId::from_string("item_1"),
                quantity: 2,
            }],
            tracking_numbers: vec!["TRACK-1".to_owned()],
            shipped_at: Some(1_700_000_000),
            partial: true,
        });

        assert_eq!(input.provider_id.as_deref(), Some("manual"));
        assert_eq!(input.items.len(), 1);
        assert_eq!(input.items[0].quantity, 2);
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
            idempotency_key: None,
        });

        assert_eq!(input.payment_id, payment_id);
        assert_eq!(input.amount, 500);
        assert_eq!(input.currency, "USD");
        assert_eq!(input.reason.as_deref(), Some("damaged"));
    }
}
