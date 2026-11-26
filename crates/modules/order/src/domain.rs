//! Order entities, status enums and service input types.
//!
//! Orders are historical records: item titles, SKUs, prices and quantities are
//! copied from the cart at placement time and are never rewritten when catalog
//! data changes. Only an explicit, recorded order edit rewrites a line's
//! quantity and amounts afterwards.

use fastcart_kernel::{
    CustomerId, FulfillmentId, LineItemId, Metadata, OrderId, PaymentId, ProductId, PromotionId,
    RefundId, Timestamp, VariantId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Lifecycle state of an order.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    /// Admin-authored order that has not entered the fulfillment lifecycle.
    Draft,
    /// Order captured but not yet confirmed.
    Pending,
    /// Order confirmed by the store.
    Confirmed,
    /// Order is being prepared.
    Processing,
    /// Order has been fulfilled.
    Fulfilled,
    /// Order is complete.
    Completed,
    /// Order was canceled.
    Canceled,
}

impl OrderStatus {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Processing => "processing",
            Self::Fulfilled => "fulfilled",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
        }
    }

    /// Parses a stored status, rejecting unknown values.
    pub fn from_stored(value: &str) -> Result<Self, fastcart_kernel::AppError> {
        match value {
            "draft" => Ok(Self::Draft),
            "pending" => Ok(Self::Pending),
            "confirmed" => Ok(Self::Confirmed),
            "processing" => Ok(Self::Processing),
            "fulfilled" => Ok(Self::Fulfilled),
            "completed" => Ok(Self::Completed),
            "canceled" => Ok(Self::Canceled),
            other => Err(fastcart_kernel::AppError::database(format!(
                "invalid order status stored in database: {other}"
            ))),
        }
    }

    /// Returns true when the order is still an editable draft.
    #[must_use]
    pub fn is_draft(self) -> bool {
        matches!(self, Self::Draft)
    }

    /// Returns true when the order may still be canceled.
    #[must_use]
    pub fn is_cancelable(self) -> bool {
        matches!(
            self,
            Self::Draft | Self::Pending | Self::Confirmed | Self::Processing
        )
    }

    /// Returns true when the order is in a terminal state.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Canceled | Self::Completed)
    }

    /// Returns true when a customer may request a return of a placed order.
    ///
    /// This is only the lifecycle half of [`Order::is_customer_returnable`];
    /// the order also has to have shipped at least partially.
    #[must_use]
    pub fn is_customer_returnable(self) -> bool {
        matches!(
            self,
            Self::Pending | Self::Confirmed | Self::Processing | Self::Fulfilled | Self::Completed
        )
    }
}

/// State of the money collection for an order.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    /// Waiting for a payment.
    Awaiting,
    /// Payment authorized but not captured.
    Authorized,
    /// Payment captured.
    Captured,
    /// Part of the captured amount was refunded.
    PartiallyRefunded,
    /// All captured money was refunded.
    Refunded,
    /// Payment failed.
    Failed,
}

impl PaymentStatus {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Awaiting => "awaiting",
            Self::Authorized => "authorized",
            Self::Captured => "captured",
            Self::PartiallyRefunded => "partially_refunded",
            Self::Refunded => "refunded",
            Self::Failed => "failed",
        }
    }

    /// Parses a stored status, rejecting unknown values.
    pub fn from_stored(value: &str) -> Result<Self, fastcart_kernel::AppError> {
        match value {
            "awaiting" => Ok(Self::Awaiting),
            "authorized" => Ok(Self::Authorized),
            "captured" => Ok(Self::Captured),
            "partially_refunded" => Ok(Self::PartiallyRefunded),
            "refunded" => Ok(Self::Refunded),
            "failed" => Ok(Self::Failed),
            other => Err(fastcart_kernel::AppError::database(format!(
                "invalid payment status stored in database: {other}"
            ))),
        }
    }
}

/// Fulfillment progress of an order.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FulfillmentStatus {
    /// Nothing has shipped.
    NotFulfilled,
    /// Some items have shipped.
    PartiallyFulfilled,
    /// Every item has shipped.
    Fulfilled,
    /// Shipped items were returned.
    Returned,
}

impl FulfillmentStatus {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFulfilled => "not_fulfilled",
            Self::PartiallyFulfilled => "partially_fulfilled",
            Self::Fulfilled => "fulfilled",
            Self::Returned => "returned",
        }
    }

    /// Parses a stored status, rejecting unknown values.
    pub fn from_stored(value: &str) -> Result<Self, fastcart_kernel::AppError> {
        match value {
            "not_fulfilled" => Ok(Self::NotFulfilled),
            "partially_fulfilled" => Ok(Self::PartiallyFulfilled),
            "fulfilled" => Ok(Self::Fulfilled),
            "returned" => Ok(Self::Returned),
            other => Err(fastcart_kernel::AppError::database(format!(
                "invalid fulfillment status stored in database: {other}"
            ))),
        }
    }
}

/// Refund progress of an order.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// Nothing was refunded.
    NotRefunded,
    /// Part of the order total was refunded.
    PartiallyRefunded,
    /// The order total was fully refunded.
    Refunded,
}

impl RefundStatus {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRefunded => "not_refunded",
            Self::PartiallyRefunded => "partially_refunded",
            Self::Refunded => "refunded",
        }
    }

    /// Parses a stored status, rejecting unknown values.
    pub fn from_stored(value: &str) -> Result<Self, fastcart_kernel::AppError> {
        match value {
            "not_refunded" => Ok(Self::NotRefunded),
            "partially_refunded" => Ok(Self::PartiallyRefunded),
            "refunded" => Ok(Self::Refunded),
            other => Err(fastcart_kernel::AppError::database(format!(
                "invalid refund status stored in database: {other}"
            ))),
        }
    }
}

/// A shipping method snapshotted onto an order at placement time.
#[derive(utoipa::ToSchema, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OrderShippingMethod {
    /// Shipping option chosen at checkout.
    pub shipping_option_id: String,
    /// Option name at purchase time.
    pub name: String,
    /// Registered fulfillment provider.
    #[serde(default = "default_shipping_provider")]
    pub provider_id: String,
    /// Amount in minor units.
    pub amount: i64,
    /// Currency of the amount.
    pub currency_code: String,
    /// Price-list style rate identifier, when a destination rate matched.
    #[serde(default)]
    pub rate_id: Option<String>,
    /// Provider-specific snapshot data.
    #[serde(default)]
    pub data: Value,
}

fn default_shipping_provider() -> String {
    "manual".to_owned()
}

/// An order with its line items and every historical commercial snapshot.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    /// Order identifier.
    pub id: OrderId,
    /// Human-readable sequential number unique within the store.
    pub display_number: i64,
    /// Optional customer identifier.
    pub customer_id: Option<CustomerId>,
    /// Contact email.
    pub email: String,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Lifecycle state.
    pub status: OrderStatus,
    /// Payment state.
    pub payment_status: PaymentStatus,
    /// Fulfillment state.
    pub fulfillment_status: FulfillmentStatus,
    /// Refund state.
    pub refund_status: RefundStatus,
    /// Sum of item subtotals in minor units.
    pub subtotal: i64,
    /// Sum of item discounts in minor units.
    pub discount_total: i64,
    /// Shipping amount in minor units.
    pub shipping_total: i64,
    /// Sum of item taxes in minor units.
    pub tax_total: i64,
    /// Order total in minor units.
    pub total: i64,
    /// Shipping address snapshot.
    pub shipping_address: Value,
    /// Billing address snapshot.
    pub billing_address: Value,
    /// Shipping methods selected at checkout, snapshotted for history.
    pub shipping_methods: Vec<OrderShippingMethod>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Store region the order was placed in, when the cart had one.
    pub region_id: Option<String>,
    /// Sales channel the order was placed through, when the cart had one.
    pub sales_channel_id: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Line items; `subtotal`, `discount_total` and `total` are their sums.
    pub items: Vec<OrderItem>,
    /// Per-rate tax breakdown; an edit scales it with the lines it is levied
    /// on, so it still adds up to `tax_total`.
    pub tax_lines: Vec<OrderTaxLine>,
    /// Order-level adjustments.
    pub adjustments: Vec<OrderAdjustment>,
    /// Status history, oldest first.
    pub status_history: Vec<OrderStatusHistory>,
    /// Fulfillment records.
    pub fulfillments: Vec<OrderFulfillment>,
    /// Refund records.
    pub refunds: Vec<OrderRefund>,
}

impl Order {
    /// Returns true when the order is an admin-authored draft that has not
    /// entered the fulfillment lifecycle.
    #[must_use]
    pub fn is_draft_order(&self) -> bool {
        self.status.is_draft()
    }

    /// Returns true when the customer may ask to return this order.
    ///
    /// Customer returns follow what the store has actually shipped. The return
    /// rule is deliberately narrower than the admin one:
    ///
    /// * The order must be live (`pending`, `confirmed`, `processing`,
    ///   `fulfilled` or `completed`). Draft orders were never placed and
    ///   canceled orders have nothing to send back.
    /// * At least one unit must have been shipped, so the fulfillment status is
    ///   `partially_fulfilled` or `fulfilled`. A `not_fulfilled` order has
    ///   nothing in transit to return, and a `returned` order is already closed.
    ///
    /// Finer checks (remaining returnable quantity per line) stay in the return
    /// service, which owns the after-sales records.
    #[must_use]
    pub fn is_customer_returnable(&self) -> bool {
        self.status.is_customer_returnable()
            && matches!(
                self.fulfillment_status,
                FulfillmentStatus::PartiallyFulfilled | FulfillmentStatus::Fulfilled
            )
    }
}

/// A tax line as it was levied when the order was placed.
///
/// Collapsing tax into `orders.tax_total` made the collected amount impossible
/// to reconcile against a rate, so invoices and filings could not be produced
/// from the order alone. An order edit scales a tax line with the line it is
/// levied on and preserves shipping-scoped tax with the shipping, so the lines
/// still sum to `orders.tax_total`.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderTaxLine {
    /// Tax line identifier.
    pub id: String,
    /// Owning order.
    pub order_id: OrderId,
    /// Line item the tax was levied on, absent for taxes levied on shipping.
    pub order_item_id: Option<LineItemId>,
    /// Shipping option the tax was levied on, absent for item taxes.
    pub shipping_option_id: Option<String>,
    /// Human-readable rate name at purchase time.
    pub name: String,
    /// Rate in basis points.
    pub rate_bps: i64,
    /// Tax amount in minor units.
    pub amount: i64,
    /// Whether the amount is already contained in the price.
    pub is_tax_inclusive: bool,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// One recalculated tax line for a draft order line.
///
/// Draft orders can be re-taxed when their destination changes. The caller
/// resolves the rate from the tax service and rewrites the item-scoped tax
/// lines in one transaction, so the order and its breakdown stay in step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftOrderTaxLineInput {
    /// Order line the tax is levied on.
    pub order_item_id: LineItemId,
    /// Human-readable rate name.
    pub name: String,
    /// Rate in basis points.
    pub rate_bps: i64,
    /// Tax amount in minor units.
    pub amount: i64,
    /// Whether the amount is already contained in the line price.
    pub is_tax_inclusive: bool,
}

/// One line of a draft as it was when its conversion priced tax and stock.
#[derive(Clone, Debug, PartialEq)]
pub struct DraftConversionItem {
    /// Order line the line describes.
    pub order_item_id: LineItemId,
    /// Variant whose inventory item holds the stock.
    pub variant_id: Option<VariantId>,
    /// Quantity the order promises.
    pub quantity: i64,
    /// Line subtotal the tax was priced on.
    pub subtotal: i64,
    /// Line discount the tax was priced on.
    pub discount_total: i64,
    /// Whether the unit price already contains tax.
    pub is_tax_inclusive: bool,
}

/// The draft state a conversion priced tax and stock from.
///
/// Conversion cannot hold a database lock while the tax provider is called, so
/// the snapshot it priced is compared again inside the conversion transaction.
/// A concurrent edit confirmation or destination change then fails the
/// conversion instead of placing a draft whose lines do not match the tax and
/// stock inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct DraftConversionSnapshot {
    /// Shipping destination the tax was priced for.
    pub shipping_address: Value,
    /// Lines the tax and stock inputs describe, in order.
    pub items: Vec<DraftConversionItem>,
}

impl DraftConversionSnapshot {
    /// Snapshots an order for conversion.
    #[must_use]
    pub fn from_order(order: &Order) -> Self {
        Self {
            shipping_address: order.shipping_address.clone(),
            items: order
                .items
                .iter()
                .map(|item| DraftConversionItem {
                    order_item_id: item.id.clone(),
                    variant_id: item.variant_id.clone(),
                    quantity: item.quantity,
                    subtotal: item.subtotal,
                    discount_total: item.discount_total,
                    is_tax_inclusive: item.is_tax_inclusive,
                })
                .collect(),
        }
    }
}

/// One line item of an order.
///
/// Titles, SKUs and prices are snapshotted at placement time; a confirmed
/// order edit is the only flow that rewrites the quantity and amounts.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderItem {
    /// Line item identifier.
    pub id: LineItemId,
    /// Owning order.
    pub order_id: OrderId,
    /// Product identifier at purchase time, when known.
    pub product_id: Option<ProductId>,
    /// Variant identifier at purchase time, when known.
    pub variant_id: Option<VariantId>,
    /// Product title at purchase time.
    pub product_title: String,
    /// Variant title at purchase time.
    pub variant_title: Option<String>,
    /// SKU at purchase time.
    pub sku: Option<String>,
    /// Thumbnail URL at purchase time.
    pub thumbnail: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Purchased quantity.
    pub quantity: i64,
    /// Line subtotal in minor units.
    pub subtotal: i64,
    /// Allocated discount in minor units.
    pub discount_total: i64,
    /// Allocated tax in minor units.
    pub tax_total: i64,
    /// Line total in minor units.
    pub total: i64,
    /// Whether the unit price already includes tax.
    pub is_tax_inclusive: bool,
    /// Whether the variant requires shipping.
    pub requires_shipping: bool,
    /// Line-level metadata snapshot.
    pub metadata: Metadata,
    /// Line-level adjustments.
    pub adjustments: Vec<OrderItemAdjustment>,
}

/// A discount or surcharge allocated to a line item.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderItemAdjustment {
    /// Adjustment identifier.
    pub id: String,
    /// Line item this adjustment belongs to.
    pub order_item_id: LineItemId,
    /// Promotion that produced the adjustment, when applicable.
    pub promotion_id: Option<PromotionId>,
    /// Promotion or adjustment code.
    pub code: Option<String>,
    /// Signed amount in minor units.
    pub amount: i64,
    /// Human-readable description.
    pub description: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// A discount or surcharge applied to the whole order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderAdjustment {
    /// Adjustment identifier.
    pub id: String,
    /// Owning order.
    pub order_id: OrderId,
    /// Adjustment code.
    pub code: String,
    /// Signed amount in minor units.
    pub amount: i64,
    /// Human-readable description.
    pub description: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// One order status transition.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderStatusHistory {
    /// History row identifier.
    pub id: String,
    /// Owning order.
    pub order_id: OrderId,
    /// Status recorded by this row.
    pub status: OrderStatus,
    /// Previous status, when known.
    pub previous_status: Option<OrderStatus>,
    /// Optional note.
    pub note: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// A quantity of one order line included in a fulfillment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderFulfillmentItem {
    /// Fulfillment item identifier.
    pub id: String,
    /// Owning fulfillment.
    pub fulfillment_id: FulfillmentId,
    /// Order line being fulfilled.
    pub order_item_id: LineItemId,
    /// Variant snapshot, when known.
    pub variant_id: Option<VariantId>,
    /// Fulfilled quantity.
    pub quantity: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// A fulfillment record for an order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderFulfillment {
    /// Fulfillment identifier.
    pub id: FulfillmentId,
    /// Owning order.
    pub order_id: OrderId,
    /// Fulfillment record status.
    pub status: String,
    /// Registered fulfillment provider.
    pub provider_id: String,
    /// Provider-side fulfillment identifier.
    pub provider_fulfillment_id: Option<String>,
    /// Provider-specific response data.
    pub provider_data: Value,
    /// Tracking numbers.
    pub tracking_numbers: Vec<String>,
    /// Shipment timestamp in Unix seconds.
    pub shipped_at: Option<Timestamp>,
    /// Delivery timestamp in Unix seconds.
    pub delivered_at: Option<Timestamp>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Fulfilled item quantities.
    pub items: Vec<OrderFulfillmentItem>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// A refund record for an order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderRefund {
    /// Refund identifier.
    pub id: RefundId,
    /// Owning order.
    pub order_id: OrderId,
    /// Payment that is refunded, when applicable.
    pub payment_id: Option<PaymentId>,
    /// Refunded amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Provider-independent refund record status.
    pub status: String,
    /// Reason for the refund.
    pub reason: Option<String>,
    /// Identifier assigned by the payment provider.
    pub provider_refund_id: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one line item when placing an order.
///
/// `subtotal` and `total` are derived when omitted. Every other title, SKU and
/// pricing field is persisted as a snapshot, later rewritable only by an
/// explicit order edit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderItemInput {
    /// Product identifier.
    pub product_id: Option<ProductId>,
    /// Variant identifier.
    pub variant_id: Option<VariantId>,
    /// Product title snapshot (required).
    pub product_title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// SKU snapshot.
    pub sku: Option<String>,
    /// Thumbnail URL snapshot.
    pub thumbnail: Option<String>,
    /// Unit price in minor units (required).
    pub unit_price: i64,
    /// Quantity (required).
    pub quantity: i64,
    /// Line subtotal; derived from unit price and quantity when omitted.
    pub subtotal: Option<i64>,
    /// Allocated discount in minor units.
    pub discount_total: Option<i64>,
    /// Allocated tax in minor units.
    pub tax_total: Option<i64>,
    /// Line total; derived from subtotal, discount and tax when omitted.
    pub total: Option<i64>,
    /// Whether the unit price includes tax.
    pub is_tax_inclusive: bool,
    /// Whether the variant requires shipping.
    pub requires_shipping: bool,
    /// Line-level metadata snapshot.
    pub metadata: Metadata,
    /// Line-level adjustments.
    pub adjustments: Vec<CreateAdjustmentInput>,
}

/// Input for an item- or order-level adjustment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateAdjustmentInput {
    /// Promotion that produced the adjustment.
    pub promotion_id: Option<PromotionId>,
    /// Adjustment code.
    pub code: Option<String>,
    /// Signed amount in minor units. Negative values are discounts.
    pub amount: i64,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Input for one tax line of [`crate::OrderService::create_order`].
///
/// The line is scoped to a purchased item by its position in `items` because
/// item identifiers are generated while the order is inserted; a line with no
/// `item_index` is levied on shipping.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderTaxLineInput {
    /// Position of the taxed item in `CreateOrderInput::items`.
    pub item_index: Option<usize>,
    /// Shipping option the tax was levied on, for a shipping tax line.
    pub shipping_option_id: Option<String>,
    /// Human-readable rate name.
    pub name: String,
    /// Rate in basis points.
    pub rate_bps: i64,
    /// Tax amount in minor units.
    pub amount: i64,
    /// Whether the amount is already contained in the price.
    pub is_tax_inclusive: bool,
}

/// Input for [`crate::OrderService::create_order`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderInput {
    /// Optional customer identifier.
    pub customer_id: Option<CustomerId>,
    /// Contact email (required).
    pub email: String,
    /// ISO-style currency code (required).
    pub currency_code: String,
    /// Purchased line items.
    pub items: Vec<CreateOrderItemInput>,
    /// Order-level adjustments.
    pub adjustments: Vec<CreateAdjustmentInput>,
    /// Shipping address snapshot.
    pub shipping_address: Value,
    /// Billing address snapshot.
    pub billing_address: Value,
    /// Shipping methods selected at checkout.
    #[serde(default)]
    pub shipping_methods: Vec<OrderShippingMethod>,
    /// Per-rate tax breakdown; its amounts must sum to `tax_total`.
    #[serde(default)]
    pub tax_lines: Vec<CreateOrderTaxLineInput>,
    /// Store region the order was placed in, when the cart had one.
    #[serde(default)]
    pub region_id: Option<String>,
    /// Sales channel the order was placed through, when the cart had one.
    #[serde(default)]
    pub sales_channel_id: Option<String>,
    /// Sum of item subtotals in minor units.
    pub subtotal: i64,
    /// Sum of item discounts in minor units.
    pub discount_total: i64,
    /// Shipping amount in minor units.
    pub shipping_total: i64,
    /// Sum of item taxes in minor units.
    pub tax_total: i64,
    /// Order total in minor units.
    pub total: i64,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Client-provided idempotency key.
    pub idempotency_key: Option<String>,
}

/// Input for [`crate::OrderService::fulfill`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FulfillmentInput {
    /// Registered fulfillment provider; defaults to `manual`.
    pub provider_id: Option<String>,
    /// Identifier assigned by the fulfillment provider, when one was created.
    #[serde(default)]
    pub provider_fulfillment_id: Option<String>,
    /// Provider-specific configuration.
    pub provider_data: Value,
    /// Items and quantities to fulfill. When empty, every remaining
    /// shippable quantity is fulfilled.
    pub items: Vec<FulfillmentItemInput>,
    /// Tracking numbers for the shipment.
    pub tracking_numbers: Vec<String>,
    /// Shipment timestamp; defaults to the current time.
    pub shipped_at: Option<Timestamp>,
    /// Marks the order as partially fulfilled instead of fully fulfilled.
    pub partial: bool,
}

/// One line and quantity to fulfill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FulfillmentItemInput {
    /// Order line being fulfilled.
    pub order_item_id: LineItemId,
    /// Quantity to fulfill.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::add_refund`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RefundInput {
    /// Payment being refunded.
    pub payment_id: PaymentId,
    /// Amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency: String,
    /// Reason for the refund.
    pub reason: Option<String>,
}

/// Filters accepted by [`crate::OrderService::list`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OrderFilter {
    /// Lifecycle state.
    pub status: Option<OrderStatus>,
    /// Payment state.
    pub payment_status: Option<PaymentStatus>,
    /// Customer identifier.
    pub customer_id: Option<CustomerId>,
    /// Contact email.
    pub email: Option<String>,
    /// Inclusive lower bound on creation time.
    pub from: Option<Timestamp>,
    /// Inclusive upper bound on creation time.
    pub to: Option<Timestamp>,
    /// Excludes admin-authored draft orders from the result.
    ///
    /// Customer-facing lists set this so an unfinished draft never appears as
    /// a purchase; the admin list leaves it off.
    pub exclude_drafts: bool,
}

#[cfg(test)]
mod tests {
    use super::{FulfillmentStatus, OrderStatus, PaymentStatus, RefundStatus};

    #[test]
    fn statuses_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&OrderStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&PaymentStatus::PartiallyRefunded).unwrap(),
            "\"partially_refunded\""
        );
        assert_eq!(
            serde_json::to_string(&FulfillmentStatus::NotFulfilled).unwrap(),
            "\"not_fulfilled\""
        );
        assert_eq!(
            serde_json::to_string(&RefundStatus::NotRefunded).unwrap(),
            "\"not_refunded\""
        );
    }

    #[test]
    fn statuses_round_trip_through_storage() {
        for status in [
            OrderStatus::Pending,
            OrderStatus::Confirmed,
            OrderStatus::Processing,
            OrderStatus::Fulfilled,
            OrderStatus::Completed,
            OrderStatus::Canceled,
        ] {
            assert_eq!(OrderStatus::from_stored(status.as_str()).unwrap(), status);
        }
        for status in [
            PaymentStatus::Awaiting,
            PaymentStatus::Authorized,
            PaymentStatus::Captured,
            PaymentStatus::PartiallyRefunded,
            PaymentStatus::Refunded,
            PaymentStatus::Failed,
        ] {
            assert_eq!(PaymentStatus::from_stored(status.as_str()).unwrap(), status);
        }
        for status in [
            FulfillmentStatus::NotFulfilled,
            FulfillmentStatus::PartiallyFulfilled,
            FulfillmentStatus::Fulfilled,
            FulfillmentStatus::Returned,
        ] {
            assert_eq!(
                FulfillmentStatus::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        for status in [
            RefundStatus::NotRefunded,
            RefundStatus::PartiallyRefunded,
            RefundStatus::Refunded,
        ] {
            assert_eq!(RefundStatus::from_stored(status.as_str()).unwrap(), status);
        }
        assert!(OrderStatus::from_stored("unknown").is_err());
    }

    #[test]
    fn cancellation_is_bounded_to_early_states() {
        assert!(OrderStatus::Pending.is_cancelable());
        assert!(OrderStatus::Confirmed.is_cancelable());
        assert!(OrderStatus::Processing.is_cancelable());
        assert!(!OrderStatus::Fulfilled.is_cancelable());
        assert!(!OrderStatus::Completed.is_cancelable());
        assert!(!OrderStatus::Canceled.is_cancelable());
    }

    #[test]
    fn customer_returns_require_a_placed_lifecycle_state() {
        assert!(!OrderStatus::Draft.is_customer_returnable());
        assert!(OrderStatus::Pending.is_customer_returnable());
        assert!(OrderStatus::Confirmed.is_customer_returnable());
        assert!(OrderStatus::Processing.is_customer_returnable());
        assert!(OrderStatus::Fulfilled.is_customer_returnable());
        assert!(OrderStatus::Completed.is_customer_returnable());
        assert!(!OrderStatus::Canceled.is_customer_returnable());
    }
}
