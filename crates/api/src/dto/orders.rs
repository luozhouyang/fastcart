use super::*;

/// One returned line in a create-return request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReturnItemRequest {
    /// Historical order line being returned.
    pub order_item_id: String,
    /// Requested return quantity.
    pub quantity: i64,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating a return.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateReturnRequest {
    /// Optional reason for the return.
    pub reason: Option<String>,
    /// Refund created for this return, when linked.
    pub refund_id: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Returned lines.
    pub items: Vec<ReturnItemRequest>,
}

/// One received line in a receive-return request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReceiveReturnItemRequest {
    /// Return line's historical order line.
    pub order_item_id: String,
    /// Incremental quantity received.
    pub quantity: i64,
}

/// Request body for receiving a return.
///
/// An empty `items` list receives every outstanding quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReceiveReturnRequest {
    /// Lines to receive.
    #[serde(default)]
    pub items: Vec<ReceiveReturnItemRequest>,
}

/// Request body for writing off received return quantities.
///
/// An empty `items` list writes off every outstanding received quantity.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WriteOffReturnRequest {
    /// Lines to write off.
    #[serde(default)]
    pub items: Vec<ReceiveReturnItemRequest>,
}

/// Request body for canceling a post-purchase operation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CancelAfterSaleRequest {
    /// Optional cancellation reason.
    pub reason: Option<String>,
}

/// One claimed line in a create-claim request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClaimItemRequest {
    /// Historical order line affected by the claim.
    pub order_item_id: String,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<String>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating a claim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateClaimRequest {
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Claimed lines.
    pub items: Vec<ClaimItemRequest>,
}

/// One exchange line in a create-exchange request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ExchangeItemRequest {
    /// Historical order line this exchange line refers to.
    pub order_item_id: String,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<String>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Request body for creating an exchange.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateExchangeRequest {
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Items returned by the customer.
    pub inbound_items: Vec<ExchangeItemRequest>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<ExchangeItemRequest>,
}

/// One edit line in a create-order-edit request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct OrderEditItemRequest {
    /// Historical order line for remove/update lines.
    pub order_item_id: Option<String>,
    /// Kind of change.
    pub action: OrderEditAction,
    /// Product identifier snapshot for added lines.
    pub product_id: Option<String>,
    /// Variant identifier snapshot for added lines.
    pub variant_id: Option<String>,
    /// Product title snapshot for added lines.
    pub product_title: Option<String>,
    /// Variant title snapshot for added lines.
    pub variant_title: Option<String>,
    /// SKU snapshot for added lines.
    pub sku: Option<String>,
    /// Unit price for added lines; ignored for existing lines.
    pub unit_price: Option<i64>,
    /// Expected current quantity for remove/update lines.
    pub before_quantity: Option<i64>,
    /// Quantity after the edit.
    pub after_quantity: i64,
    /// Whether the added line requires shipping; defaults to true.
    pub requires_shipping: Option<bool>,
}

/// Request body for creating an order edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateOrderEditRequest {
    /// Optional reason for the edit.
    pub reason: Option<String>,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Edit lines.
    pub items: Vec<OrderEditItemRequest>,
}

/// One line of a draft order creation request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DraftOrderItemRequest {
    /// Product identifier snapshot.
    pub product_id: Option<String>,
    /// Variant identifier snapshot.
    pub variant_id: Option<String>,
    /// Product title snapshot.
    pub product_title: String,
    /// Variant title snapshot.
    pub variant_title: Option<String>,
    /// SKU snapshot.
    pub sku: Option<String>,
    /// Thumbnail URL snapshot.
    pub thumbnail: Option<String>,
    /// Unit price in minor units.
    pub unit_price: i64,
    /// Quantity to buy.
    pub quantity: i64,
    /// Whether the unit price already contains tax.
    #[serde(default)]
    pub is_tax_inclusive: bool,
    /// Whether the line requires shipping; defaults to true.
    #[serde(default = "default_true")]
    pub requires_shipping: bool,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

fn default_true() -> bool {
    true
}

/// Request body for creating an admin-authored draft order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateDraftOrderRequest {
    /// Optional customer the order is for.
    pub customer_id: Option<String>,
    /// Contact email.
    pub email: String,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Store region the order belongs to.
    pub region_id: Option<String>,
    /// Sales channel the order belongs to.
    pub sales_channel_id: Option<String>,
    /// Lines to buy.
    pub items: Vec<DraftOrderItemRequest>,
    /// Shipping address snapshot; a country_code enables tax pricing.
    #[serde(default)]
    pub shipping_address: serde_json::Value,
    /// Billing address snapshot.
    #[serde(default)]
    pub billing_address: serde_json::Value,
    /// Shipping amount in minor units.
    #[serde(default)]
    pub shipping_total: i64,
    /// Free-form metadata.
    #[serde(default)]
    pub metadata: Metadata,
    /// Optional idempotency key for retried creations.
    pub idempotency_key: Option<String>,
}

/// Request body for changing a draft order's destination.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateDraftAddressRequest {
    /// New shipping address snapshot; its country_code drives tax.
    pub shipping_address: serde_json::Value,
    /// New billing address snapshot, left unchanged when omitted.
    pub billing_address: Option<serde_json::Value>,
}

/// A page of the authenticated customer's orders.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct OrderListResponse {
    /// Orders in this page.
    pub orders: Vec<Order>,
    /// Total number of matching orders.
    pub count: i64,
    /// Number of orders skipped.
    pub offset: i64,
    /// Maximum number of orders requested.
    pub limit: i64,
}

impl OrderListResponse {
    /// Builds an order envelope from a kernel page.
    #[must_use]
    pub fn from_page(page: PageResult<Order>) -> Self {
        let Pagination { limit, offset } = page.pagination;
        Self {
            orders: page.data,
            count: page.count,
            offset,
            limit,
        }
    }
}

/// A single order owned by the authenticated customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct OrderResponse {
    /// The requested order.
    pub order: Order,
}
