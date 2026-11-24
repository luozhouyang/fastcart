//! Order aggregate reads and persistence row decoding.

use std::collections::HashMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CustomerId, FulfillmentId, LineItemId, Metadata, OrderId, Pagination, PaymentId,
    ProductId, RefundId, VariantId, normalize_email,
};
use serde_json::Value;

use crate::domain::{
    FulfillmentStatus, Order, OrderAdjustment, OrderFilter, OrderFulfillment, OrderFulfillmentItem,
    OrderItem, OrderItemAdjustment, OrderRefund, OrderShippingMethod, OrderStatus,
    OrderStatusHistory, OrderTaxLine, PaymentStatus, RefundStatus,
};

use super::{IDEMPOTENCY_SCOPE, OrderService};

/// Persistence row for `orders`.
#[derive(sqlx::FromRow)]
struct OrderRow {
    id: String,
    display_number: i64,
    customer_id: Option<String>,
    email: String,
    currency_code: String,
    status: String,
    payment_status: String,
    fulfillment_status: String,
    refund_status: String,
    subtotal: i64,
    discount_total: i64,
    shipping_total: i64,
    tax_total: i64,
    total: i64,
    shipping_address: String,
    billing_address: String,
    shipping_methods: String,
    metadata: String,
    region_id: Option<String>,
    sales_channel_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    canceled_at: Option<i64>,
}

impl OrderRow {
    #[allow(clippy::too_many_arguments)]
    fn into_order(
        self,
        items: Vec<OrderItem>,
        tax_lines: Vec<OrderTaxLine>,
        adjustments: Vec<OrderAdjustment>,
        status_history: Vec<OrderStatusHistory>,
        fulfillments: Vec<OrderFulfillment>,
        refunds: Vec<OrderRefund>,
    ) -> Result<Order, AppError> {
        Ok(Order {
            id: OrderId::from_string(self.id),
            display_number: self.display_number,
            customer_id: self.customer_id.map(CustomerId::from_string),
            email: self.email,
            currency_code: self.currency_code,
            status: OrderStatus::from_stored(&self.status)?,
            payment_status: PaymentStatus::from_stored(&self.payment_status)?,
            fulfillment_status: FulfillmentStatus::from_stored(&self.fulfillment_status)?,
            refund_status: RefundStatus::from_stored(&self.refund_status)?,
            subtotal: self.subtotal,
            discount_total: self.discount_total,
            shipping_total: self.shipping_total,
            tax_total: self.tax_total,
            total: self.total,
            shipping_address: json_value(&self.shipping_address),
            billing_address: json_value(&self.billing_address),
            shipping_methods: parse_shipping_methods(&self.shipping_methods)?,
            metadata: Metadata::from_json_str(&self.metadata),
            region_id: self.region_id,
            sales_channel_id: self.sales_channel_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
            canceled_at: self.canceled_at,
            items,
            tax_lines,
            adjustments,
            status_history,
            fulfillments,
            refunds,
        })
    }
}

/// Persistence row for `order_items`.
#[derive(sqlx::FromRow)]
struct OrderItemRow {
    id: String,
    order_id: String,
    product_id: Option<String>,
    variant_id: Option<String>,
    product_title: String,
    variant_title: Option<String>,
    sku: Option<String>,
    thumbnail: Option<String>,
    unit_price: i64,
    quantity: i64,
    subtotal: i64,
    discount_total: i64,
    tax_total: i64,
    total: i64,
    is_tax_inclusive: i64,
    requires_shipping: i64,
    metadata: String,
}

impl OrderItemRow {
    fn into_domain(self, adjustments: Vec<OrderItemAdjustment>) -> OrderItem {
        OrderItem {
            id: LineItemId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            product_id: self.product_id.map(ProductId::from_string),
            variant_id: self.variant_id.map(VariantId::from_string),
            product_title: self.product_title,
            variant_title: self.variant_title,
            sku: self.sku,
            thumbnail: self.thumbnail,
            unit_price: self.unit_price,
            quantity: self.quantity,
            subtotal: self.subtotal,
            discount_total: self.discount_total,
            tax_total: self.tax_total,
            total: self.total,
            is_tax_inclusive: self.is_tax_inclusive != 0,
            requires_shipping: self.requires_shipping != 0,
            metadata: Metadata::from_json_str(&self.metadata),
            adjustments,
        }
    }
}

/// Persistence row for `order_item_adjustments`.
#[derive(sqlx::FromRow)]
struct ItemAdjustmentRow {
    id: String,
    order_item_id: String,
    promotion_id: Option<String>,
    code: Option<String>,
    amount: i64,
    description: Option<String>,
    created_at: i64,
}

impl ItemAdjustmentRow {
    fn into_domain(self) -> OrderItemAdjustment {
        OrderItemAdjustment {
            id: self.id,
            order_item_id: LineItemId::from_string(self.order_item_id),
            promotion_id: self
                .promotion_id
                .map(fastcart_kernel::PromotionId::from_string),
            code: self.code,
            amount: self.amount,
            description: self.description,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_tax_lines`.
#[derive(sqlx::FromRow)]
struct OrderTaxLineRow {
    id: String,
    order_id: String,
    order_item_id: Option<String>,
    shipping_option_id: Option<String>,
    name: String,
    rate_bps: i64,
    amount: i64,
    is_tax_inclusive: i64,
    created_at: i64,
}

impl OrderTaxLineRow {
    fn into_domain(self) -> OrderTaxLine {
        OrderTaxLine {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            order_item_id: self.order_item_id.map(LineItemId::from_string),
            shipping_option_id: self.shipping_option_id,
            name: self.name,
            rate_bps: self.rate_bps,
            amount: self.amount,
            is_tax_inclusive: self.is_tax_inclusive != 0,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_adjustments`.
#[derive(sqlx::FromRow)]
struct AdjustmentRow {
    id: String,
    order_id: String,
    code: String,
    amount: i64,
    description: Option<String>,
    created_at: i64,
}

impl AdjustmentRow {
    fn into_domain(self) -> OrderAdjustment {
        OrderAdjustment {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            code: self.code,
            amount: self.amount,
            description: self.description,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_status_history`.
#[derive(sqlx::FromRow)]
pub(super) struct HistoryRow {
    id: String,
    order_id: String,
    status: String,
    previous_status: Option<String>,
    note: Option<String>,
    created_at: i64,
}

impl HistoryRow {
    pub(super) fn into_domain(self) -> Result<OrderStatusHistory, AppError> {
        let previous_status = match self.previous_status {
            Some(value) => Some(OrderStatus::from_stored(&value)?),
            None => None,
        };
        Ok(OrderStatusHistory {
            id: self.id,
            order_id: OrderId::from_string(self.order_id),
            status: OrderStatus::from_stored(&self.status)?,
            previous_status,
            note: self.note,
            created_at: self.created_at,
        })
    }
}

/// Persistence row for `order_fulfillments`.
#[derive(sqlx::FromRow)]
struct FulfillmentRow {
    id: String,
    order_id: String,
    status: String,
    provider_id: String,
    provider_fulfillment_id: Option<String>,
    provider_data: String,
    tracking_numbers: String,
    shipped_at: Option<i64>,
    delivered_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl FulfillmentRow {
    fn into_domain(self) -> OrderFulfillment {
        OrderFulfillment {
            id: FulfillmentId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            status: self.status,
            provider_id: self.provider_id,
            provider_fulfillment_id: self.provider_fulfillment_id,
            provider_data: json_value(&self.provider_data),
            tracking_numbers: json_string_list(&self.tracking_numbers),
            shipped_at: self.shipped_at,
            delivered_at: self.delivered_at,
            metadata: Metadata::from_json_str(&self.metadata),
            items: Vec::new(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Persistence row for `order_fulfillment_items`.
#[derive(sqlx::FromRow)]
struct FulfillmentItemRow {
    id: String,
    fulfillment_id: String,
    order_item_id: String,
    variant_id: Option<String>,
    quantity: i64,
    created_at: i64,
}

impl FulfillmentItemRow {
    fn into_domain(self) -> OrderFulfillmentItem {
        OrderFulfillmentItem {
            id: self.id,
            fulfillment_id: FulfillmentId::from_string(self.fulfillment_id),
            order_item_id: LineItemId::from_string(self.order_item_id),
            variant_id: self.variant_id.map(VariantId::from_string),
            quantity: self.quantity,
            created_at: self.created_at,
        }
    }
}

/// Persistence row for `order_refunds`.
#[derive(sqlx::FromRow)]
struct RefundRow {
    id: String,
    order_id: String,
    payment_id: Option<String>,
    amount: i64,
    currency_code: String,
    status: String,
    reason: Option<String>,
    provider_refund_id: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl RefundRow {
    fn into_domain(self) -> OrderRefund {
        OrderRefund {
            id: RefundId::from_string(self.id),
            order_id: OrderId::from_string(self.order_id),
            payment_id: self.payment_id.map(PaymentId::from_string),
            amount: self.amount,
            currency_code: self.currency_code,
            status: self.status,
            reason: self.reason,
            provider_refund_id: self.provider_refund_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

mod queries;

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

/// A bound value used by the dynamic order filter.
enum FilterBind {
    Text(String),
    Integer(i64),
}

fn build_filter(filter: &OrderFilter) -> (String, Vec<FilterBind>) {
    let mut conditions = Vec::new();
    let mut binds = Vec::new();

    if let Some(status) = filter.status {
        conditions.push("status = ?");
        binds.push(FilterBind::Text(status.as_str().to_owned()));
    }
    if let Some(payment_status) = filter.payment_status {
        conditions.push("payment_status = ?");
        binds.push(FilterBind::Text(payment_status.as_str().to_owned()));
    }
    if let Some(customer_id) = &filter.customer_id {
        conditions.push("customer_id = ?");
        binds.push(FilterBind::Text(customer_id.as_str().to_owned()));
    }
    if let Some(email) = &filter.email {
        conditions.push("email = ?");
        binds.push(FilterBind::Text(normalize_email(email)));
    }
    if let Some(from) = filter.from {
        conditions.push("created_at >= ?");
        binds.push(FilterBind::Integer(from));
    }
    if let Some(to) = filter.to {
        conditions.push("created_at <= ?");
        binds.push(FilterBind::Integer(to));
    }

    if conditions.is_empty() {
        (String::new(), binds)
    } else {
        (format!(" WHERE {}", conditions.join(" AND ")), binds)
    }
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn json_string_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

fn parse_shipping_methods(text: &str) -> Result<Vec<OrderShippingMethod>, AppError> {
    serde_json::from_str(text)
        .map_err(|error| AppError::internal(format!("invalid shipping methods: {error}")))
}
