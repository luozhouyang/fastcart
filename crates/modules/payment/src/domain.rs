//! Payment entities, statuses, inputs and pure state-transition rules.
//!
//! Money is always represented as an integer number of minor units together
//! with a currency code. No floating point values appear in this module.

use fastcart_kernel::{
    CartId, CheckoutId, Metadata, OrderId, PaymentAttemptId, PaymentId, PaymentSessionId, RefundId,
    WebhookId,
};
use serde::{Deserialize, Serialize};

/// Lifecycle status of a payment session, attempt or payment.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    /// The session exists but no action has been taken yet.
    Pending,
    /// The customer must complete an additional action (for example a redirect).
    RequiresAction,
    /// The provider authorized the amount without capturing it.
    Authorized,
    /// Some, but not all, of the session amount was captured.
    ///
    /// The remaining amount may still be captured and the captured amount is
    /// refundable.
    PartiallyCaptured,
    /// The amount was captured.
    Captured,
    /// Some, but not all, of the captured amount was refunded.
    PartiallyRefunded,
    /// The full captured amount was refunded.
    Refunded,
    /// The payment or session failed.
    Failed,
    /// The payment or session was canceled before capture.
    Canceled,
}

impl PaymentStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::RequiresAction => "requires_action",
            Self::Authorized => "authorized",
            Self::PartiallyCaptured => "partially_captured",
            Self::Captured => "captured",
            Self::PartiallyRefunded => "partially_refunded",
            Self::Refunded => "refunded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "requires_action" => Some(Self::RequiresAction),
            "authorized" => Some(Self::Authorized),
            "partially_captured" => Some(Self::PartiallyCaptured),
            "captured" => Some(Self::Captured),
            "partially_refunded" => Some(Self::PartiallyRefunded),
            "refunded" => Some(Self::Refunded),
            "failed" => Some(Self::Failed),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }

    /// Returns true when no further transitions are allowed.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Canceled | Self::Refunded)
    }

    /// Returns true when the transition is permitted by the payment state
    /// machine.
    ///
    /// A transition to the same state is always allowed so that idempotent
    /// operations can be retried safely.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Pending => matches!(
                next,
                Self::RequiresAction
                    | Self::Authorized
                    | Self::PartiallyCaptured
                    | Self::Captured
                    | Self::Failed
                    | Self::Canceled
            ),
            Self::RequiresAction => matches!(
                next,
                Self::Authorized
                    | Self::PartiallyCaptured
                    | Self::Captured
                    | Self::Failed
                    | Self::Canceled
            ),
            Self::Authorized => matches!(
                next,
                Self::PartiallyCaptured | Self::Captured | Self::Failed | Self::Canceled
            ),
            // A partially captured session may still be completed or refunded,
            // but it holds captured money, so failing or canceling it (which
            // would drop that money from the session state) is not a valid
            // transition.
            Self::PartiallyCaptured => matches!(
                next,
                Self::PartiallyCaptured | Self::Captured | Self::PartiallyRefunded | Self::Refunded
            ),
            Self::Captured => matches!(next, Self::PartiallyRefunded | Self::Refunded),
            Self::PartiallyRefunded => matches!(next, Self::PartiallyRefunded | Self::Refunded),
            Self::Failed | Self::Canceled | Self::Refunded => false,
        }
    }
}

impl std::fmt::Display for PaymentStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Lifecycle status of a refund.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundStatus {
    /// The refund was accepted but not confirmed yet.
    Pending,
    /// The provider confirmed the refund.
    Succeeded,
    /// The refund failed.
    Failed,
}

impl RefundStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// Processing status of a persisted webhook event.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookStatus {
    /// The event was persisted but not processed yet.
    Received,
    /// The event was applied successfully.
    Processed,
    /// Processing failed; the error is stored on the record.
    Failed,
}

impl WebhookStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Processed => "processed",
            Self::Failed => "failed",
        }
    }

    /// Parses a stored status, falling back to `Received` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "processed" => Self::Processed,
            "failed" => Self::Failed,
            _ => Self::Received,
        }
    }
}

/// A provider-backed payment session belonging to a cart, checkout or order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaymentSession {
    /// Session identifier.
    pub id: PaymentSessionId,
    /// Owning cart, when the session was created during cart checkout.
    pub cart_id: Option<CartId>,
    /// Owning checkout, when the session was created during checkout.
    pub checkout_id: Option<CheckoutId>,
    /// Owning order, when the session is associated with an order.
    pub order_id: Option<OrderId>,
    /// Registered provider identifier.
    pub provider: String,
    /// Provider-assigned session identifier.
    pub provider_session_id: Option<String>,
    /// Amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Current lifecycle status.
    pub status: PaymentStatus,
    /// Provider-independent extension data.
    pub data: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// One attempt to create, authorize or capture a payment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaymentAttempt {
    /// Attempt identifier.
    pub id: PaymentAttemptId,
    /// Session this attempt belongs to.
    pub payment_session_id: PaymentSessionId,
    /// Attempted amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Attempt status.
    pub status: PaymentStatus,
    /// Provider reference (session, payment or refund identifier).
    pub provider_reference: Option<String>,
    /// Failure detail, when the attempt failed.
    pub error: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// A captured payment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Payment {
    /// Payment identifier.
    pub id: PaymentId,
    /// Owning order, when known.
    pub order_id: Option<OrderId>,
    /// Registered provider identifier.
    pub provider: String,
    /// Provider-assigned payment identifier.
    pub provider_payment_id: Option<String>,
    /// Captured amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Current lifecycle status.
    pub status: PaymentStatus,
    /// Capture timestamp in Unix seconds.
    pub captured_at: Option<i64>,
    /// Provider-independent extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A refund against a captured payment.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Refund {
    /// Refund identifier.
    pub id: RefundId,
    /// Payment that was refunded.
    pub payment_id: Option<PaymentId>,
    /// Owning order, when known.
    pub order_id: Option<OrderId>,
    /// Refunded amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Refund status.
    pub status: RefundStatus,
    /// Provider-assigned refund identifier.
    pub provider_refund_id: Option<String>,
    /// Caller-supplied idempotency key, retained for replay.
    pub idempotency_key: Option<String>,
    /// Human-readable refund reason.
    pub reason: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A persisted webhook event used for replay protection and audit trails.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaymentWebhookRecord {
    /// Local record identifier.
    pub id: WebhookId,
    /// Registered provider identifier.
    pub provider: String,
    /// Provider-assigned event identifier.
    pub provider_event_id: String,
    /// Stable event type, for example `payment.captured`.
    pub event_type: String,
    /// Serialized event data.
    pub payload: String,
    /// Processing status.
    pub status: WebhookStatus,
    /// Number of processing attempts.
    pub attempts: i64,
    /// Processing timestamp in Unix seconds.
    pub processed_at: Option<i64>,
    /// Last processing error, when processing failed.
    pub last_error: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// Identifier of a payment collection.
///
/// Collections only exist inside the payment module, so the identifier lives
/// here instead of `fastcart_kernel`; it still follows the kernel identifier
/// shape (`"{prefix}_{uuid}"` with a UUID v7 suffix).
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct PaymentCollectionId(String);

impl PaymentCollectionId {
    /// Creates a new identifier with a UUID v7 suffix.
    #[must_use]
    pub fn new() -> Self {
        Self(format!("pcoll_{}", uuid::Uuid::now_v7().simple()))
    }

    /// Wraps an existing identifier string without validation.
    #[must_use]
    pub fn from_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PaymentCollectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PaymentCollectionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<String> for PaymentCollectionId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for PaymentCollectionId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Lifecycle status of a payment collection.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentCollectionStatus {
    /// Nothing has been captured against the collection yet.
    Open,
    /// Some, but not all, of the collection amount was captured.
    PartiallyCaptured,
    /// The captured amount covers the collection amount.
    Captured,
}

impl PaymentCollectionStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PartiallyCaptured => "partially_captured",
            Self::Captured => "captured",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "partially_captured" => Some(Self::PartiallyCaptured),
            "captured" => Some(Self::Captured),
            _ => None,
        }
    }

    /// Derives the status from the amount captured against the collection.
    #[must_use]
    pub fn from_captured(amount: i64, captured: i64) -> Self {
        if captured <= 0 {
            Self::Open
        } else if captured >= amount {
            Self::Captured
        } else {
            Self::PartiallyCaptured
        }
    }
}

impl std::fmt::Display for PaymentCollectionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The money a checkout or order owes, paid by one or more payment sessions.
///
/// A collection is the anchor for combination payments: a single checkout can
/// be paid with several sessions, and it is only settled once the captured
/// amounts add up to [`PaymentCollection::amount`].
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaymentCollection {
    /// Collection identifier.
    pub id: PaymentCollectionId,
    /// Owning checkout, when the collection was opened by a checkout.
    pub checkout_id: Option<CheckoutId>,
    /// Owning order, when the collection was associated with an order.
    pub order_id: Option<OrderId>,
    /// Amount due in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Status last recorded by the service; [`crate::PaymentService::collection_status`]
    /// derives the live value from the amounts actually captured.
    pub status: PaymentCollectionStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Lifecycle status of a gift card.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GiftCardStatus {
    /// The card can still be redeemed.
    Active,
    /// The card was disabled and rejects redemption until it is enabled again.
    Disabled,
}

impl GiftCardStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

impl std::fmt::Display for GiftCardStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A prepaid card whose remaining balance pays for future purchases.
///
/// Gift cards are issued in one currency and never change it: a card may only
/// be redeemed against money priced in its own currency.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GiftCard {
    /// Gift card identifier.
    pub id: String,
    /// Opaque redemption code. Treat it as a bearer credential: an administrator
    /// reads it to hand it to the customer, so admin responses return it in
    /// full.
    pub code: String,
    /// ISO-style currency code the card is priced in.
    pub currency_code: String,
    /// Balance the card was issued with, in minor units.
    pub initial_amount_minor: i64,
    /// Remaining balance in minor units.
    pub balance_minor: i64,
    /// Current lifecycle status.
    pub status: GiftCardStatus,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// Input for [`crate::PaymentService::issue_gift_card`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreateGiftCardInput {
    /// ISO-style currency code.
    pub currency_code: String,
    /// Initial balance in minor units; must be positive.
    pub amount_minor: i64,
    /// Caller-supplied redemption code; a random code is generated when absent.
    pub code: Option<String>,
    /// Extension metadata.
    pub metadata: Metadata,
}

impl CreateGiftCardInput {
    /// Creates an input with a generated code and no metadata.
    #[must_use]
    pub fn new(currency_code: impl Into<String>, amount_minor: i64) -> Self {
        Self {
            currency_code: currency_code.into(),
            amount_minor,
            code: None,
            metadata: Metadata::empty(),
        }
    }
}

/// One gift card's contribution to a checkout, recorded when it is redeemed.
///
/// The ledger row is written in the same transaction as the guarded balance
/// decrement, keyed uniquely by `(checkout_id, gift_card_id)`, so a retried
/// completion returns the recorded redemption instead of spending the card
/// twice. `order_id` is filled in once the order is placed, which is what makes
/// the redemption queryable from the order and the admin.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiftCardRedemption {
    /// Redemption identifier.
    pub id: String,
    /// Gift card that paid.
    pub gift_card_id: String,
    /// Checkout the redemption belongs to.
    pub checkout_id: String,
    /// Order the checkout was placed as, once it exists.
    pub order_id: Option<String>,
    /// Amount taken from the card, in minor units.
    pub amount_minor: i64,
    /// `redeemed` while the money is spent; `reversed` after a compensation.
    pub status: GiftCardRedemptionStatus,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
    /// Timestamp the redemption was reversed, when it was.
    pub reversed_at: Option<i64>,
}

/// Lifecycle of one gift card redemption.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GiftCardRedemptionStatus {
    /// The card paid and the money is spent.
    Redeemed,
    /// The card was credited back after a failed completion.
    Reversed,
}

impl GiftCardRedemptionStatus {
    /// Stable storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redeemed => "redeemed",
            Self::Reversed => "reversed",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "redeemed" => Some(Self::Redeemed),
            "reversed" => Some(Self::Reversed),
            _ => None,
        }
    }
}

/// One credit a refund wrote back to a gift card.
///
/// A credit is the reverse of a [`GiftCardRedemption`]: it names the
/// redemption whose money the refund restored and the card the money landed
/// on. Rows are unique per `(refund_id, redemption_id)`, so replaying a refund
/// can never credit the same redemption twice, and a partial refund leaves one
/// row per redemption it touched.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiftCardRefundCredit {
    /// Credit identifier.
    pub id: String,
    /// Refund that restored the money.
    pub refund_id: String,
    /// Redemption whose money the refund gave back.
    pub redemption_id: String,
    /// Gift card the money was credited to.
    pub gift_card_id: String,
    /// Amount credited to the card, in minor units.
    pub amount_minor: i64,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
}

/// Input for [`crate::PaymentService::create_session`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CreatePaymentSessionInput {
    /// Owning cart.
    pub cart_id: Option<CartId>,
    /// Owning checkout.
    pub checkout_id: Option<CheckoutId>,
    /// Owning order.
    pub order_id: Option<OrderId>,
    /// Registered provider identifier.
    pub provider: String,
    /// Amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Provider-independent extension data.
    pub metadata: Metadata,
}

impl CreatePaymentSessionInput {
    /// Creates an input with the required fields and no associations.
    #[must_use]
    pub fn new(provider: impl Into<String>, amount: i64, currency_code: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            amount,
            currency_code: currency_code.into(),
            ..Self::default()
        }
    }
}

/// Filter for [`crate::PaymentService::list_payments`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListPaymentsFilter {
    /// Restricts results to payments for one order.
    pub order_id: Option<OrderId>,
    /// Restricts results to one status.
    pub status: Option<PaymentStatus>,
    /// Offset/limit pagination.
    pub pagination: fastcart_kernel::Pagination,
}

impl ListPaymentsFilter {
    /// Creates an empty filter with default pagination.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts results to an order.
    #[must_use]
    pub fn with_order(mut self, order_id: OrderId) -> Self {
        self.order_id = Some(order_id);
        self
    }

    /// Restricts results to a status.
    #[must_use]
    pub fn with_status(mut self, status: PaymentStatus) -> Self {
        self.status = Some(status);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{PaymentCollectionStatus, PaymentStatus};

    #[test]
    fn pending_can_reach_every_non_refund_state() {
        for next in [
            PaymentStatus::RequiresAction,
            PaymentStatus::Authorized,
            PaymentStatus::Captured,
            PaymentStatus::Failed,
            PaymentStatus::Canceled,
            PaymentStatus::Pending,
        ] {
            assert!(PaymentStatus::Pending.can_transition_to(next), "{next}");
        }
        assert!(!PaymentStatus::Pending.can_transition_to(PaymentStatus::Refunded));
        assert!(!PaymentStatus::Pending.can_transition_to(PaymentStatus::PartiallyRefunded));
    }

    #[test]
    fn captured_only_transitions_through_refunds() {
        assert!(PaymentStatus::Captured.can_transition_to(PaymentStatus::PartiallyRefunded));
        assert!(PaymentStatus::Captured.can_transition_to(PaymentStatus::Refunded));
        assert!(!PaymentStatus::Captured.can_transition_to(PaymentStatus::Pending));
        assert!(!PaymentStatus::Captured.can_transition_to(PaymentStatus::Canceled));
    }

    #[test]
    fn terminal_states_only_allow_same_state() {
        for terminal in [
            PaymentStatus::Failed,
            PaymentStatus::Canceled,
            PaymentStatus::Refunded,
        ] {
            assert!(terminal.is_terminal());
            assert!(terminal.can_transition_to(terminal));
            assert!(!terminal.can_transition_to(PaymentStatus::Captured));
        }
        assert!(PaymentStatus::PartiallyRefunded.can_transition_to(PaymentStatus::Refunded));
        assert!(
            PaymentStatus::PartiallyRefunded.can_transition_to(PaymentStatus::PartiallyRefunded)
        );
    }

    #[test]
    fn payment_collection_status_round_trips_and_follows_the_captured_amount() {
        for status in [
            PaymentCollectionStatus::Open,
            PaymentCollectionStatus::PartiallyCaptured,
            PaymentCollectionStatus::Captured,
        ] {
            assert_eq!(
                PaymentCollectionStatus::parse(status.as_str()),
                Some(status)
            );
        }
        assert_eq!(PaymentCollectionStatus::parse("unknown"), None);

        assert_eq!(
            PaymentCollectionStatus::from_captured(1_000, 0),
            PaymentCollectionStatus::Open
        );
        assert_eq!(
            PaymentCollectionStatus::from_captured(1_000, 400),
            PaymentCollectionStatus::PartiallyCaptured
        );
        assert_eq!(
            PaymentCollectionStatus::from_captured(1_000, 1_000),
            PaymentCollectionStatus::Captured
        );
    }

    #[test]
    fn statuses_round_trip_through_storage_representation() {
        for status in [
            PaymentStatus::Pending,
            PaymentStatus::RequiresAction,
            PaymentStatus::Authorized,
            PaymentStatus::Captured,
            PaymentStatus::PartiallyRefunded,
            PaymentStatus::Refunded,
            PaymentStatus::Failed,
            PaymentStatus::Canceled,
        ] {
            assert_eq!(PaymentStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(PaymentStatus::parse("unknown"), None);
    }
}
