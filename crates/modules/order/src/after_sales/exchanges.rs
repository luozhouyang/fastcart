use super::*;

/// An exchange for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderExchange {
    /// Exchange identifier.
    pub id: ExchangeId,
    /// Owning order.
    pub order_id: OrderId,
    /// Exchange lifecycle state.
    pub status: ExchangeStatus,
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Price difference of the exchange's outbound lines, settled on
    /// acceptance: positive is owed by the customer, negative is owed to them.
    /// `None` until the exchange is accepted.
    #[serde(default)]
    pub difference_total: Option<i64>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Acceptance timestamp in Unix seconds.
    pub accepted_at: Option<Timestamp>,
    /// Cancellation timestamp in Unix seconds.
    pub canceled_at: Option<Timestamp>,
    /// Items returned by the customer.
    pub inbound_items: Vec<OrderExchangeItem>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<OrderExchangeItem>,
}

/// One line of an exchange.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderExchangeItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning exchange.
    pub exchange_id: ExchangeId,
    /// Historical order line this exchange line refers to.
    pub order_item_id: LineItemId,
    /// Inbound or outbound direction.
    pub direction: ExchangeItemDirection,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Price difference this line settled on acceptance, when it settled one:
    /// `(replacement unit price - replaced unit price) * quantity`. Positive is
    /// owed by the customer, negative is owed to them. `None` for inbound
    /// lines, for records that are not accepted yet, and for replacements the
    /// caller resolved no price for.
    #[serde(default)]
    pub difference: Option<i64>,
    /// Line state.
    pub status: ExchangeItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one exchange line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExchangeItemInput {
    /// Historical order line this exchange line refers to.
    pub order_item_id: LineItemId,
    /// Exchanged quantity.
    pub quantity: i64,
    /// Replacement variant for outbound lines.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_exchange`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateExchangeInput {
    /// Optional reason for the exchange.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Items returned by the customer.
    pub inbound_items: Vec<ExchangeItemInput>,
    /// Replacement items shipped to the customer.
    pub outbound_items: Vec<ExchangeItemInput>,
}

/// The unit price the caller resolved for one replacement line.
///
/// The order module does not price catalog variants, so the caller that
/// confirms an exchange or claim resolves the replacement's unit price and
/// supplies it through [`crate::OrderService::accept_exchange`] and
/// [`crate::OrderService::accept_claim`]. The line is named by the historical
/// order line it replaces, which is unique within one after-sales record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementUnitPrice {
    /// Historical order line the replacement stands in for.
    pub order_item_id: LineItemId,
    /// Unit price of the replacement in minor units.
    pub unit_price: i64,
}
