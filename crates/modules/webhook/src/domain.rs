//! Webhook endpoints, deliveries and retry policy.

use fastcart_kernel::{Metadata, Timestamp, WebhookDeliveryId, WebhookId};
use serde::{Deserialize, Serialize};

/// Default number of delivery attempts before a delivery is marked failed.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 8;
/// Largest accepted endpoint URL.
pub const MAX_URL_LENGTH: usize = 2_048;
/// Largest number of event subscriptions on one endpoint.
pub const MAX_EVENTS: usize = 100;
/// Largest accepted event type name.
pub const MAX_EVENT_LENGTH: usize = 128;
/// Response bodies retained for diagnostics are truncated to this size.
pub const MAX_RESPONSE_BODY_BYTES: usize = 2_048;

/// A registered webhook endpoint. The signing secret is intentionally absent.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebhookEndpoint {
    /// Endpoint identifier.
    pub id: WebhookId,
    /// Delivery URL.
    pub url: String,
    /// Subscribed event types, or `*` for every event.
    pub events: Vec<String>,
    /// Whether deliveries are currently sent.
    pub is_active: bool,
    /// Free-form metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
}

/// Result of creating or rotating an endpoint secret.
///
/// The secret is returned exactly once; list and get responses never include
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedWebhookEndpoint {
    /// Endpoint without its secret.
    pub endpoint: WebhookEndpoint,
    /// Plaintext signing secret.
    pub secret: String,
}

/// Input for registering a webhook endpoint.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CreateWebhookEndpointInput {
    /// Delivery URL.
    pub url: String,
    /// Subscribed event types, or `*`.
    pub events: Vec<String>,
    /// Optional caller-supplied signing secret. A random secret is generated
    /// when omitted.
    pub secret: Option<String>,
    /// Optional metadata.
    pub metadata: Option<Metadata>,
}

/// Input for updating a webhook endpoint.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct UpdateWebhookEndpointInput {
    /// Replacement delivery URL.
    pub url: Option<String>,
    /// Replacement event subscriptions.
    pub events: Option<Vec<String>>,
    /// Enables or disables deliveries.
    pub is_active: Option<bool>,
}

/// Delivery lifecycle.
#[derive(utoipa::ToSchema, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Waiting for its next attempt.
    Pending,
    /// Claimed by a worker.
    Delivering,
    /// The endpoint accepted the event.
    Delivered,
    /// All attempts failed.
    Failed,
}

impl DeliveryStatus {
    /// Storage and API representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }

    /// Parses a stored status value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "delivered" => Some(Self::Delivered),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl std::fmt::Display for DeliveryStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One event delivery to one endpoint.
#[derive(utoipa::ToSchema, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WebhookDelivery {
    /// Delivery identifier.
    pub id: WebhookDeliveryId,
    /// Target endpoint.
    pub endpoint_id: WebhookId,
    /// Source domain event.
    pub event_id: String,
    /// Event type.
    pub event_type: String,
    /// Signed event envelope.
    pub payload: serde_json::Value,
    /// Lifecycle status.
    pub status: DeliveryStatus,
    /// Attempts made so far.
    pub attempts: i64,
    /// Last response status code.
    pub response_status: Option<i64>,
    /// Truncated last response body.
    pub response_body: Option<String>,
    /// Earliest next attempt.
    pub next_retry_at: Option<Timestamp>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Last update timestamp.
    pub updated_at: Timestamp,
}

/// Counters produced by one delivery sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    /// Deliveries claimed.
    pub attempted: u64,
    /// Deliveries accepted by their endpoint.
    pub delivered: u64,
    /// Deliveries rescheduled for a later attempt.
    pub retried: u64,
    /// Deliveries that exhausted their attempts.
    pub failed: u64,
    /// Deliveries skipped because their endpoint disappeared or was disabled.
    pub skipped: u64,
}

/// Backoff in seconds for the given attempt number.
#[must_use]
pub fn retry_delay_seconds(attempts: i64) -> i64 {
    let exponent = attempts.clamp(1, 12);
    let delay =
        30_i64.saturating_mul(2_i64.saturating_pow(u32::try_from(exponent - 1).unwrap_or(0)));
    delay.min(3_600)
}
