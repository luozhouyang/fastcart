use super::*;

/// A claim for one order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderClaim {
    /// Claim identifier.
    pub id: ClaimId,
    /// Owning order.
    pub order_id: OrderId,
    /// Claim lifecycle state.
    pub status: ClaimStatus,
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Refund created for this claim, when linked.
    #[serde(default)]
    pub refund_id: Option<RefundId>,
    /// Price difference of the claim's replacement lines, settled on
    /// acceptance: positive is owed by the customer, negative is owed to them.
    /// `None` until the claim is accepted.
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
    /// Claimed lines.
    pub items: Vec<OrderClaimItem>,
}

/// One line of a claim.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderClaimItem {
    /// Line identifier.
    pub id: EventId,
    /// Owning claim.
    pub claim_id: ClaimId,
    /// Historical order line affected by the claim.
    pub order_item_id: LineItemId,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
    /// Price difference this replacement settled on acceptance, when it settled
    /// one: `(replacement unit price - replaced unit price) * quantity`.
    /// Positive is owed by the customer, negative is owed to them. `None` for
    /// refund lines, for records that are not accepted yet, and for
    /// replacements the caller resolved no price for.
    #[serde(default)]
    pub difference: Option<i64>,
    /// Line state.
    pub status: ClaimItemStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for one claimed line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClaimItemInput {
    /// Historical order line affected by the claim.
    pub order_item_id: LineItemId,
    /// Claimed quantity.
    pub quantity: i64,
    /// Refund or replacement resolution.
    pub item_type: ClaimItemType,
    /// Replacement variant for `replace` claims.
    pub replacement_variant_id: Option<VariantId>,
    /// Optional line-level reason.
    pub reason: Option<String>,
}

/// Input for [`crate::OrderService::create_claim`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateClaimInput {
    /// Optional reason for the claim.
    pub reason: Option<String>,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Claimed lines.
    pub items: Vec<ClaimItemInput>,
}
