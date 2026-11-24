use super::*;

/// A provider-side fulfillment created for an accepted claim or exchange.
///
/// Exactly one of `claim_id` and `exchange_id` is set. Replacement stock
/// movements and provider calls are logically owned by this record, which is
/// why only one fulfillment is allowed per after-sales record.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AfterSalesFulfillment {
    /// Fulfillment identifier.
    pub id: FulfillmentId,
    /// Owning order.
    pub order_id: OrderId,
    /// Claim this fulfillment replaces items for, when applicable.
    pub claim_id: Option<ClaimId>,
    /// Exchange this fulfillment replaces items for, when applicable.
    pub exchange_id: Option<ExchangeId>,
    /// Fulfillment record status.
    pub status: String,
    /// Registered fulfillment provider.
    pub provider_id: String,
    /// Provider-side fulfillment identifier.
    pub provider_fulfillment_id: Option<String>,
    /// Provider-specific response data.
    pub provider_data: Value,
    /// Tracking numbers known to the provider.
    pub tracking_numbers: Vec<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
    /// Replacement quantities shipped.
    pub items: Vec<AfterSalesFulfillmentItem>,
}

/// One replacement variant quantity in an after-sales fulfillment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AfterSalesFulfillmentItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning fulfillment.
    pub fulfillment_id: FulfillmentId,
    /// Replacement variant shipped.
    pub variant_id: VariantId,
    /// Shipped quantity.
    pub quantity: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// One replacement variant quantity to persist on a fulfillment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterSalesFulfillmentItemInput {
    /// Replacement variant shipped.
    pub variant_id: VariantId,
    /// Shipped quantity.
    pub quantity: i64,
}

/// Input for [`crate::OrderService::create_after_sales_fulfillment`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateAfterSalesFulfillmentInput {
    /// Owning order.
    pub order_id: OrderId,
    /// Claim this fulfillment replaces items for, when applicable.
    pub claim_id: Option<ClaimId>,
    /// Exchange this fulfillment replaces items for, when applicable.
    pub exchange_id: Option<ExchangeId>,
    /// Registered fulfillment provider.
    pub provider_id: String,
    /// Provider-side fulfillment identifier.
    pub provider_fulfillment_id: Option<String>,
    /// Provider-specific response data.
    pub provider_data: Value,
    /// Tracking numbers known to the provider.
    pub tracking_numbers: Vec<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Replacement quantities shipped.
    pub items: Vec<AfterSalesFulfillmentItemInput>,
}
