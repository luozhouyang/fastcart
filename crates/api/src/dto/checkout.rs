use super::*;

/// A cart line item as described by the public contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = LineItem)]
pub struct LineItemDto {
    /// Line item identifier.
    pub id: String,
    /// Owning cart, when the line belongs to one.
    pub cart_id: Option<String>,
    /// Referenced variant.
    pub variant_id: String,
    /// Product title snapshot.
    #[schema(required = false)]
    pub title: String,
    /// Quantity of units.
    pub quantity: i64,
    /// Unit price in minor units.
    #[schema(required = false)]
    pub unit_price: i64,
    /// Total price in minor units.
    #[schema(required = false)]
    pub total_price: i64,
    /// Free-form metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
}

/// Commercial totals summary reported by a checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Totals)]
pub struct TotalsDto {
    /// Currency of every amount.
    pub currency_code: String,
    /// Subtotal before discounts.
    pub subtotal: i64,
    /// Total discounts applied.
    #[schema(required = false)]
    pub discount_total: i64,
    /// Total shipping charges.
    #[schema(required = false)]
    pub shipping_total: i64,
    /// Total tax.
    #[schema(required = false)]
    pub tax_total: i64,
    /// Payable total.
    pub grand_total: i64,
}

/// Lifecycle state of the checkout summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutSummaryStatus {
    /// Created but not completed.
    Pending,
    /// Fully processed.
    Completed,
    /// Canceled before completion.
    Canceled,
    /// Expired before completion.
    Expired,
}

/// A checkout summary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Checkout)]
pub struct CheckoutDto {
    /// Checkout identifier.
    pub id: String,
    /// Cart being checked out.
    pub cart_id: String,
    /// Current lifecycle state.
    pub status: CheckoutSummaryStatus,
    /// Optional contact email.
    pub email: Option<String>,
    /// Payment provider handling the checkout.
    #[schema(required = false)]
    pub payment_provider: String,
    /// Payment session attached to the checkout, if any.
    pub payment_session_id: Option<String>,
    /// Commercial totals snapshot.
    #[schema(required = false)]
    pub totals: TotalsDto,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Expiry timestamp in Unix seconds.
    pub expires_at: Option<i64>,
}

/// The current storefront cart.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CartResponse {
    /// Cart contents and totals.
    pub cart: Cart,
}

/// A checkout session.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CheckoutResponse {
    /// Checkout contents and state.
    pub checkout: CheckoutSession,
}

/// Result of completing a checkout, as a Medusa v2 discriminated union.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CheckoutCompleteResponse {
    /// Discriminator identifying an order result.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The placed order.
    pub order: Order,
}

impl CheckoutCompleteResponse {
    /// Wraps a placed order in the completion envelope.
    #[must_use]
    pub fn order(order: Order) -> Self {
        Self {
            event_type: "order".to_owned(),
            order,
        }
    }
}

/// The state transition history of a checkout.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CheckoutHistoryResponse {
    /// History entries in chronological order.
    pub history: Vec<CheckoutStateHistory>,
}
