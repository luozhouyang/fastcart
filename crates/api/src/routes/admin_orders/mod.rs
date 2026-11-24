//! Admin order HTTP routes.
//!

//! Handlers translate HTTP requests into [`OrderService`] calls after enforcing
//! the `orders.read`, `orders.write` and `orders.cancel` permissions.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};
use fastcart_http::{ApiError, ApiResult};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    ActorContext, AppError, CustomerId, LineItemId, PaymentId, ProductId, RefundId, VariantId,
};
use fastcart_module_order::{
    AfterSalesFulfillment, ClaimItemInput, CreateClaimInput, CreateExchangeInput,
    CreateOrderEditInput, CreateReturnInput, ExchangeItemInput, FulfillmentInput,
    FulfillmentItemInput, Order, OrderClaim, OrderEditItemInput, OrderExchange, OrderFilter,
    OrderItem, OrderRefund, OrderReturn, OrderStatus, PaymentStatus, ReceiveReturnInput,
    ReceiveReturnItemInput, RefundInput, ReturnItemInput, WriteOffReturnInput,
    WriteOffReturnItemInput,
};
use fastcart_module_payment::{PaymentCollection, PaymentService, PaymentSession, Refund};
use fastcart_module_shipping::FulfillmentItemRequest as ProviderFulfillmentItemRequest;
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{
    CreateClaimRequest, CreateExchangeRequest, CreateOrderEditRequest, CreateReturnRequest,
    ReceiveReturnRequest, WriteOffReturnRequest,
};
use crate::state::AppContext;

/// Builds the routes for this namespace.
mod claims;
mod edits;
mod exchanges;
mod fulfillment;
mod orders;
mod refunds;
mod returns;

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/orders", get(orders::list_orders))
        .route("/orders/{id}", get(orders::get_order))
        .route(
            "/orders/by-number/{display_number}",
            get(orders::get_order_by_number),
        )
        .route("/orders/{id}/confirm", post(orders::confirm_order))
        .route("/orders/{id}/process", post(orders::process_order))
        .route("/orders/{id}/cancel", post(orders::cancel_order))
        .route("/orders/{id}/fulfill", post(fulfillment::fulfill_order))
        .route("/orders/{id}/history", get(orders::order_history))
        .route(
            "/orders/{id}/refunds",
            get(refunds::list_refunds).post(refunds::create_refund),
        )
        .route(
            "/orders/{id}/returns",
            get(returns::list_returns).post(returns::create_return),
        )
        .route("/orders/{id}/returns/{return_id}", get(returns::get_return))
        .route(
            "/orders/{id}/returns/{return_id}/request",
            post(returns::request_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/receive",
            post(returns::receive_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/refund",
            post(returns::refund_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/write-off",
            post(returns::write_off_return),
        )
        .route(
            "/orders/{id}/returns/{return_id}/cancel",
            post(returns::cancel_return),
        )
        .route(
            "/orders/{id}/claims",
            get(claims::list_claims).post(claims::create_claim),
        )
        .route("/orders/{id}/claims/{claim_id}", get(claims::get_claim))
        .route(
            "/orders/{id}/claims/{claim_id}/accept",
            post(claims::accept_claim),
        )
        .route(
            "/orders/{id}/claims/{claim_id}/fulfill",
            post(claims::fulfill_claim),
        )
        .route(
            "/orders/{id}/claims/{claim_id}/refund",
            post(claims::refund_claim),
        )
        .route(
            "/orders/{id}/claims/{claim_id}/cancel",
            post(claims::cancel_claim),
        )
        .route(
            "/orders/{id}/exchanges",
            get(exchanges::list_exchanges).post(exchanges::create_exchange),
        )
        .route(
            "/orders/{id}/exchanges/{exchange_id}",
            get(exchanges::get_exchange),
        )
        .route(
            "/orders/{id}/exchanges/{exchange_id}/accept",
            post(exchanges::accept_exchange),
        )
        .route(
            "/orders/{id}/exchanges/{exchange_id}/fulfill",
            post(exchanges::fulfill_exchange),
        )
        .route(
            "/orders/{id}/exchanges/{exchange_id}/cancel",
            post(exchanges::cancel_exchange),
        )
        .route(
            "/orders/{id}/edits",
            get(edits::list_order_edits).post(edits::create_order_edit),
        )
        .route("/orders/{id}/edits/{edit_id}", get(edits::get_order_edit))
        .route(
            "/orders/{id}/edits/{edit_id}/confirm",
            post(edits::confirm_order_edit),
        )
        .route(
            "/orders/{id}/edits/{edit_id}/collect-difference",
            post(edits::collect_order_edit_difference),
        )
        .route(
            "/orders/{id}/edits/{edit_id}/refund-difference",
            post(edits::refund_order_edit_difference),
        )
        .route(
            "/orders/{id}/edits/{edit_id}/cancel",
            post(edits::cancel_order_edit),
        )
        .route("/orders/{id}/payment-summary", get(edits::payment_summary))
}

/// Builds the documented routes for this namespace.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(orders::list_orders))
        .routes(routes!(orders::get_order))
        .routes(routes!(orders::get_order_by_number))
        .routes(routes!(orders::confirm_order))
        .routes(routes!(orders::process_order))
        .routes(routes!(orders::cancel_order))
        .routes(routes!(fulfillment::fulfill_order))
        .routes(routes!(orders::order_history))
        .routes(routes!(refunds::list_refunds, refunds::create_refund))
        .routes(routes!(returns::list_returns, returns::create_return))
        .routes(routes!(returns::get_return))
        .routes(routes!(returns::request_return))
        .routes(routes!(returns::receive_return))
        .routes(routes!(returns::refund_return))
        .routes(routes!(returns::write_off_return))
        .routes(routes!(returns::cancel_return))
        .routes(routes!(claims::list_claims, claims::create_claim))
        .routes(routes!(claims::get_claim))
        .routes(routes!(claims::accept_claim))
        .routes(routes!(claims::fulfill_claim))
        .routes(routes!(claims::refund_claim))
        .routes(routes!(claims::collect_claim_difference))
        .routes(routes!(claims::refund_claim_difference))
        .routes(routes!(claims::cancel_claim))
        .routes(routes!(
            exchanges::list_exchanges,
            exchanges::create_exchange
        ))
        .routes(routes!(exchanges::get_exchange))
        .routes(routes!(exchanges::accept_exchange))
        .routes(routes!(exchanges::fulfill_exchange))
        .routes(routes!(exchanges::collect_exchange_difference))
        .routes(routes!(exchanges::refund_exchange_difference))
        .routes(routes!(exchanges::cancel_exchange))
        .routes(routes!(edits::list_order_edits, edits::create_order_edit))
        .routes(routes!(edits::get_order_edit))
        .routes(routes!(edits::confirm_order_edit))
        .routes(routes!(edits::collect_order_edit_difference))
        .routes(routes!(edits::refund_order_edit_difference))
        .routes(routes!(edits::cancel_order_edit))
        .routes(routes!(edits::payment_summary))
}

// -------------------------------------------------------------------------
// Requests
// -------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct OrderListQuery {
    status: Option<OrderStatus>,
    payment_status: Option<PaymentStatus>,
    customer_id: Option<CustomerId>,
    email: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CancelOrderRequest {
    reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct FulfillOrderRequest {
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

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct FulfillmentItemRequest {
    order_item_id: LineItemId,
    quantity: i64,
}

#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct RefundOrderRequest {
    payment_id: PaymentId,
    amount: i64,
    currency: String,
    reason: Option<String>,
    idempotency_key: Option<String>,
}

/// Result of refunding a return.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ReturnRefundResponse {
    /// Order after the refund was recorded.
    pub order: Order,
    /// Return with its refund link populated.
    pub order_return: OrderReturn,
    /// Local order refund row, when one was created.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when one was attempted.
    pub provider_refund: Option<Refund>,
}

/// Result of refunding an accepted claim.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ClaimRefundResponse {
    /// Order after the refund was recorded.
    pub order: Order,
    /// Accepted claim with its refund link populated.
    pub claim: OrderClaim,
    /// Local order refund row, when one was created.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when one was attempted.
    pub provider_refund: Option<Refund>,
}

/// Request body for fulfilling an accepted claim or exchange.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct FulfillAfterSalesRequest {
    /// Registered fulfillment provider; defaults to `manual`.
    #[serde(default)]
    pub provider_id: Option<String>,
    /// Tracking numbers for the replacement shipment.
    #[serde(default)]
    pub tracking_numbers: Vec<String>,
}

/// Result of fulfilling a claim's replacement lines.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ClaimFulfillmentResponse {
    /// Order after the fulfillment.
    pub order: Order,
    /// Accepted claim the replacement belongs to.
    pub claim: OrderClaim,
    /// Persisted after-sales fulfillment, when replacement lines existed.
    pub fulfillment: Option<AfterSalesFulfillment>,
    /// Replacement units consumed from inventory.
    pub consumed_units: i64,
}

/// Result of fulfilling an exchange's outbound lines.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ExchangeFulfillmentResponse {
    /// Order after the fulfillment.
    pub order: Order,
    /// Accepted exchange the replacement belongs to.
    pub exchange: OrderExchange,
    /// Persisted after-sales fulfillment, when outbound lines existed.
    pub fulfillment: Option<AfterSalesFulfillment>,
    /// Replacement units consumed from inventory.
    pub consumed_units: i64,
}

// -------------------------------------------------------------------------
// Pure request mapping
// -------------------------------------------------------------------------

/// Builds a payment service scoped to the administrator's selected store.
fn payment_service(
    context: &AppContext,
    actor: &ActorContext,
    db: &StoreDatabase,
) -> ApiResult<PaymentService> {
    let store_id = actor
        .store_id
        .as_ref()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    Ok(PaymentService::with_registry(
        db.clone(),
        context.payment_providers_for(store_id),
    ))
}

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
        provider_fulfillment_id: None,
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

/// Maps the requested lines onto the variants snapshotted on the order.
///
/// When the request has no explicit lines the remaining shippable quantity of
/// every line is used. Quantities already recorded in previous fulfillments are
/// subtracted so a retry never asks the provider to ship the same units twice.
fn map_provider_items(
    order: &Order,
    requested: &[FulfillmentItemRequest],
) -> ApiResult<Vec<ProviderFulfillmentItemRequest>> {
    let mut fulfilled: HashMap<&str, i64> = HashMap::new();
    for fulfillment in &order.fulfillments {
        for item in &fulfillment.items {
            *fulfilled.entry(item.order_item_id.as_str()).or_default() += item.quantity;
        }
    }
    let remaining = |item: &OrderItem| -> i64 {
        if !item.requires_shipping {
            return 0;
        }
        item.quantity
            .saturating_sub(fulfilled.get(item.id.as_str()).copied().unwrap_or_default())
    };
    let variant_id = |item: &OrderItem| -> String {
        item.variant_id
            .as_ref()
            .map_or_else(|| item.id.as_str().to_owned(), |id| id.as_str().to_owned())
    };

    if requested.is_empty() {
        return Ok(order
            .items
            .iter()
            .filter(|item| remaining(item) > 0)
            .map(|item| ProviderFulfillmentItemRequest {
                variant_id: variant_id(item),
                quantity: remaining(item),
            })
            .collect());
    }

    let mut seen = HashSet::new();
    let mut items = Vec::with_capacity(requested.len());
    for line in requested {
        if line.quantity <= 0 {
            return Err(ApiError::validation(
                "fulfillment item quantities must be positive",
            ));
        }
        if !seen.insert(line.order_item_id.as_str()) {
            return Err(ApiError::validation(
                "fulfillment items must not repeat an order item",
            ));
        }
        let Some(order_item) = order
            .items
            .iter()
            .find(|order_item| order_item.id == line.order_item_id)
        else {
            return Err(ApiError::validation(
                "fulfillment item does not belong to this order",
            ));
        };
        if !order_item.requires_shipping {
            return Err(ApiError::validation(
                "fulfillment item does not require shipping",
            ));
        }
        let left = remaining(order_item);
        if line.quantity > left {
            return Err(ApiError::validation(format!(
                "cannot fulfill {} units; only {left} remain",
                line.quantity
            )));
        }
        items.push(ProviderFulfillmentItemRequest {
            variant_id: variant_id(order_item),
            quantity: line.quantity,
        });
    }
    Ok(items)
}

/// Whether a provider response carries data worth persisting.
fn has_provider_data(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) => !map.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
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
                requires_shipping: item.requires_shipping,
            })
            .collect(),
    }
}

// -------------------------------------------------------------------------
// Queries
// -------------------------------------------------------------------------

/// The provider a difference is collected through.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CollectDifferenceRequest {
    /// Registered payment provider identifier.
    pub provider: String,
}

/// The payable an after-sales difference was collected as.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct DifferenceCollectionResponse {
    /// Order aggregate the difference belongs to.
    pub order: Order,
    /// Collection that expects the difference.
    pub payment_collection: PaymentCollection,
    /// Session that pays the difference.
    pub payment_session: PaymentSession,
}

/// An after-sales difference paid back to the customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct DifferenceRefundResponse {
    /// Order aggregate after the refund.
    pub order: Order,
    /// Order refund row recorded for the difference, when one was written.
    pub order_refund: Option<OrderRefund>,
    /// Provider-side refund, when one was attempted.
    pub provider_refund: Option<Refund>,
}

/// The money a confirmed edit left between the order and the customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct PaymentSummaryResponse {
    /// Order the summary belongs to.
    pub order_id: String,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Current order total in minor units.
    pub total: i64,
    /// Money actually captured for the order.
    pub captured_total: i64,
    /// Money refunded, including refunds still pending at the provider.
    pub refunded_total: i64,
    /// Captured minus refunded.
    pub net_paid: i64,
    /// Order total minus net paid. Positive is owed by the customer, negative
    /// is owed back to them.
    pub outstanding: i64,
    /// Money owed back to the customer, when `outstanding` is negative.
    pub refund_due: i64,
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
