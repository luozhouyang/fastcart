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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    /// The session exists but no action has been taken yet.
    Pending,
    /// The customer must complete an additional action (for example a redirect).
    RequiresAction,
    /// The provider authorized the amount without capturing it.
    Authorized,
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
                    | Self::Captured
                    | Self::Failed
                    | Self::Canceled
            ),
            Self::RequiresAction => matches!(
                next,
                Self::Authorized | Self::Captured | Self::Failed | Self::Canceled
            ),
            Self::Authorized => {
                matches!(next, Self::Captured | Self::Failed | Self::Canceled)
            }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Human-readable refund reason.
    pub reason: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

/// A persisted webhook event used for replay protection and audit trails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    use super::PaymentStatus;

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
