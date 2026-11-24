use super::*;

/// A signature attached to a webhook payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WebhookSignature {
    /// Signature value.
    pub signature: String,
    /// Signature timestamp in Unix seconds.
    pub timestamp: i64,
    /// Signature algorithm.
    pub algorithm: String,
}

/// An outbound webhook envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WebhookEnvelope {
    /// Delivery identifier.
    pub id: String,
    /// Event type.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Store the event belongs to.
    pub store_id: String,
    /// Event payload.
    pub data: Value,
    /// Optional payload signature.
    pub signature: Option<WebhookSignature>,
}

/// An inbound payment-provider webhook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PaymentWebhook {
    /// Payment provider name.
    pub provider: String,
    /// Provider-side event identifier.
    pub provider_event_id: String,
    /// Provider event type.
    pub event_type: String,
    /// Provider payload.
    pub payload: Value,
    /// Optional payload signature.
    pub signature: Option<WebhookSignature>,
}
