//! Checkout entities, the pure state machine and service input types.

use fastcart_kernel::{
    AppError, CartId, CheckoutId, CustomerId, OrderId, PaymentSessionId, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Lifecycle state of a checkout session.
///
/// The checkout module only models the state machine. The actual validation,
/// pricing and payment work is orchestrated by workflows that call into the
/// cart, pricing, payment and order modules.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutStatus {
    /// The session has been created but not validated yet.
    Created,
    /// The session is being validated.
    Validating,
    /// All commercial data is valid and payment may be prepared.
    ReadyForPayment,
    /// A payment session is pending confirmation.
    PaymentPending,
    /// Payment has been confirmed.
    PaymentConfirmed,
    /// An order has been placed for this checkout.
    OrderPlaced,
    /// The checkout failed.
    Failed,
    /// The checkout was canceled.
    Canceled,
}

impl CheckoutStatus {
    /// Stable storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Validating => "validating",
            Self::ReadyForPayment => "ready_for_payment",
            Self::PaymentPending => "payment_pending",
            Self::PaymentConfirmed => "payment_confirmed",
            Self::OrderPlaced => "order_placed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// Parses a stored status, rejecting unknown values.
    pub fn from_stored(value: &str) -> Result<Self, AppError> {
        match value {
            "created" => Ok(Self::Created),
            "validating" => Ok(Self::Validating),
            "ready_for_payment" => Ok(Self::ReadyForPayment),
            "payment_pending" => Ok(Self::PaymentPending),
            "payment_confirmed" => Ok(Self::PaymentConfirmed),
            "order_placed" => Ok(Self::OrderPlaced),
            "failed" => Ok(Self::Failed),
            "canceled" => Ok(Self::Canceled),
            other => Err(AppError::database(format!(
                "invalid checkout status stored in database: {other}"
            ))),
        }
    }

    /// Returns true when the state machine allows a transition to `next`.
    ///
    /// Terminal states (`Failed`, `Canceled` and `OrderPlaced`) never transition.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Created => matches!(next, Self::Validating | Self::Failed | Self::Canceled),
            Self::Validating => {
                matches!(next, Self::ReadyForPayment | Self::Failed | Self::Canceled)
            }
            Self::ReadyForPayment => {
                matches!(next, Self::PaymentPending | Self::Failed | Self::Canceled)
            }
            Self::PaymentPending => {
                matches!(next, Self::PaymentConfirmed | Self::Failed | Self::Canceled)
            }
            Self::PaymentConfirmed => matches!(next, Self::OrderPlaced | Self::Failed),
            Self::Failed | Self::Canceled | Self::OrderPlaced => false,
        }
    }

    /// Returns true when the session is in a terminal state.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Canceled | Self::OrderPlaced)
    }

    /// Returns true when details may still be changed.
    #[must_use]
    pub fn can_update_details(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Validating | Self::ReadyForPayment
        )
    }

    /// Returns true when a stale session in this state may be expired.
    #[must_use]
    pub fn is_expirable(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Validating | Self::ReadyForPayment | Self::PaymentPending
        )
    }
}

/// A checkout session converting a cart into an order.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckoutSession {
    /// Checkout identifier.
    pub id: CheckoutId,
    /// Cart being checked out.
    pub cart_id: CartId,
    /// Optional customer identifier.
    pub customer_id: Option<CustomerId>,
    /// Optional contact email.
    pub email: Option<String>,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Current lifecycle state.
    pub status: CheckoutStatus,
    /// Shipping address snapshot.
    pub shipping_address: Value,
    /// Billing address snapshot.
    pub billing_address: Value,
    /// Selected shipping method snapshot.
    pub shipping_method: Value,
    /// Commercial totals snapshot.
    pub totals: Value,
    /// Applied promotion codes.
    pub promotion_codes: Vec<String>,
    /// Payment session attached to the checkout, if any.
    pub payment_session_id: Option<PaymentSessionId>,
    /// Order created from this checkout, if any.
    pub order_id: Option<OrderId>,
    /// Client-provided idempotency key.
    pub idempotency_key: Option<String>,
    /// Expiry timestamp in Unix seconds.
    pub expires_at: Option<Timestamp>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// One recorded checkout state transition.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckoutStateHistory {
    /// History row identifier.
    pub id: String,
    /// Checkout this row belongs to.
    pub checkout_id: CheckoutId,
    /// Previous state, absent for the initial creation row.
    pub from_state: Option<CheckoutStatus>,
    /// New state.
    pub to_state: CheckoutStatus,
    /// Human-readable reason or note.
    pub reason: Option<String>,
    /// Timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// Input for [`crate::CheckoutService::create`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateCheckoutInput {
    /// Cart being converted.
    pub cart_id: CartId,
    /// Optional customer identifier.
    pub customer_id: Option<CustomerId>,
    /// Optional contact email.
    pub email: Option<String>,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Shipping address snapshot.
    pub shipping_address: Value,
    /// Billing address snapshot.
    pub billing_address: Value,
    /// Selected shipping method snapshot.
    pub shipping_method: Value,
    /// Commercial totals snapshot.
    pub totals: Value,
    /// Applied promotion codes.
    pub promotion_codes: Vec<String>,
    /// Client-provided idempotency key.
    pub idempotency_key: Option<String>,
    /// Lifetime of the session in seconds.
    pub expires_in_seconds: i64,
}

/// Input for [`crate::CheckoutService::update_details`].
///
/// Every field is optional; only supplied fields are changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateCheckoutDetailsInput {
    /// Replacement contact email.
    pub email: Option<String>,
    /// Replacement shipping address snapshot.
    pub shipping_address: Option<Value>,
    /// Replacement billing address snapshot.
    pub billing_address: Option<Value>,
    /// Replacement shipping method snapshot.
    pub shipping_method: Option<Value>,
    /// Replacement totals snapshot.
    pub totals: Option<Value>,
    /// Replacement promotion codes.
    pub promotion_codes: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::CheckoutStatus;

    #[test]
    fn state_machine_allows_only_documented_transitions() {
        assert!(CheckoutStatus::Created.can_transition_to(CheckoutStatus::Validating));
        assert!(CheckoutStatus::Created.can_transition_to(CheckoutStatus::Failed));
        assert!(CheckoutStatus::Created.can_transition_to(CheckoutStatus::Canceled));
        assert!(!CheckoutStatus::Created.can_transition_to(CheckoutStatus::ReadyForPayment));
        assert!(!CheckoutStatus::Created.can_transition_to(CheckoutStatus::Created));

        assert!(CheckoutStatus::Validating.can_transition_to(CheckoutStatus::ReadyForPayment));
        assert!(CheckoutStatus::ReadyForPayment.can_transition_to(CheckoutStatus::PaymentPending));
        assert!(CheckoutStatus::PaymentPending.can_transition_to(CheckoutStatus::PaymentConfirmed));
        assert!(CheckoutStatus::PaymentConfirmed.can_transition_to(CheckoutStatus::OrderPlaced));
        assert!(!CheckoutStatus::PaymentConfirmed.can_transition_to(CheckoutStatus::Canceled));
    }

    #[test]
    fn terminal_states_never_transition() {
        for status in [
            CheckoutStatus::Failed,
            CheckoutStatus::Canceled,
            CheckoutStatus::OrderPlaced,
        ] {
            assert!(status.is_terminal());
            for next in [
                CheckoutStatus::Created,
                CheckoutStatus::Validating,
                CheckoutStatus::ReadyForPayment,
                CheckoutStatus::PaymentPending,
                CheckoutStatus::PaymentConfirmed,
                CheckoutStatus::OrderPlaced,
                CheckoutStatus::Failed,
                CheckoutStatus::Canceled,
            ] {
                assert!(!status.can_transition_to(next));
            }
        }
    }

    #[test]
    fn statuses_round_trip_through_storage() {
        for status in [
            CheckoutStatus::Created,
            CheckoutStatus::Validating,
            CheckoutStatus::ReadyForPayment,
            CheckoutStatus::PaymentPending,
            CheckoutStatus::PaymentConfirmed,
            CheckoutStatus::OrderPlaced,
            CheckoutStatus::Failed,
            CheckoutStatus::Canceled,
        ] {
            assert_eq!(
                CheckoutStatus::from_stored(status.as_str()).unwrap(),
                status
            );
        }
        assert!(CheckoutStatus::from_stored("unknown").is_err());
    }
}
