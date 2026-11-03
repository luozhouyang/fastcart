//! Provider-independent payment interface and the default sandbox provider.
//!
//! Providers own all external communication. Application services call them
//! outside database transactions and persist the resulting identifiers.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fastcart_kernel::{AppError, Metadata};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::domain::{PaymentStatus, RefundStatus};

/// Errors raised by a payment provider.
///
/// Crypto and secret details are deliberately excluded from these messages.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PaymentError {
    /// The provider rejected the request or could not be reached.
    #[error("payment provider error: {0}")]
    Provider(String),
    /// The request is structurally invalid for this provider.
    #[error("invalid payment request: {0}")]
    InvalidRequest(String),
    /// The webhook signature could not be verified.
    #[error("webhook signature verification failed")]
    InvalidSignature,
}

impl From<PaymentError> for AppError {
    fn from(error: PaymentError) -> Self {
        match error {
            PaymentError::Provider(message) => Self::ExternalService(message),
            PaymentError::InvalidRequest(message) => Self::Validation(message),
            PaymentError::InvalidSignature => Self::Unauthorized,
        }
    }
}

/// Request to create a provider-side payment session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateProviderSession {
    /// Local reference, normally the payment session identifier.
    pub reference: String,
    /// Amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Provider-independent extension metadata.
    pub metadata: Metadata,
}

/// Provider-side session created for a local payment session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSession {
    /// Provider-assigned session identifier.
    pub provider_session_id: String,
    /// Initial status, normally `pending` or `requires_action`.
    pub status: PaymentStatus,
    /// Opaque provider data that may be returned to a storefront.
    pub data: serde_json::Value,
}

/// Request to capture a previously created provider session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureRequest {
    /// Provider-assigned session identifier.
    pub provider_session_id: String,
    /// Amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Provider-independent extension metadata.
    pub metadata: Metadata,
}

/// Result of a successful capture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaptureResult {
    /// Provider-assigned payment identifier.
    pub provider_payment_id: String,
    /// Captured amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Resulting status.
    pub status: PaymentStatus,
}

/// Request to refund a captured provider payment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefundRequest {
    /// Provider-assigned payment identifier.
    pub provider_payment_id: String,
    /// Refund amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Human-readable refund reason.
    pub reason: Option<String>,
    /// Optional key used by the provider to deduplicate retries.
    pub idempotency_key: Option<String>,
    /// Provider-independent extension metadata.
    pub metadata: Metadata,
}

/// Result of an accepted refund.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefundResult {
    /// Provider-assigned refund identifier.
    pub provider_refund_id: String,
    /// Refunded amount in minor units.
    pub amount: i64,
    /// ISO-style currency code.
    pub currency_code: String,
    /// Resulting refund status.
    pub status: RefundStatus,
}

/// A webhook whose signature has been verified by the provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VerifiedWebhook {
    /// Provider-assigned event identifier, used for replay protection.
    pub event_id: String,
    /// Stable event type, for example `payment.captured`.
    pub event_type: String,
    /// Verified event data.
    pub data: serde_json::Value,
}

/// External payment provider contract.
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Stable provider identifier used by payment sessions.
    fn id(&self) -> &str;

    /// Creates a provider-side payment session.
    async fn create_session(
        &self,
        request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError>;

    /// Captures an authorized or pending provider session.
    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError>;

    /// Refunds a captured provider payment.
    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError>;

    /// Verifies a raw webhook request and extracts its event.
    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError>;
}

type HmacSha256 = Hmac<Sha256>;

/// Header carrying the hexadecimal HMAC signature for the manual provider.
pub const MANUAL_SIGNATURE_HEADER: &str = "x-manual-signature";

/// Sandbox provider used by tests and local development.
///
/// Identifiers are random so retries never collide. Webhook verification uses
/// HMAC-SHA256 over the raw request body with the configured secret; when no
/// secret is configured every webhook is rejected.
pub struct ManualProvider {
    id: String,
    secret: Option<String>,
}

impl ManualProvider {
    /// Creates a manual provider with an optional webhook secret.
    #[must_use]
    pub fn new(id: impl Into<String>, secret: Option<String>) -> Self {
        Self {
            id: id.into(),
            secret,
        }
    }

    /// Creates the default secretless sandbox provider.
    ///
    /// Webhook delivery is rejected until a secret is configured.
    #[must_use]
    pub fn sandbox() -> Self {
        Self::new("manual", None)
    }

    fn random_hex() -> String {
        let mut bytes = [0_u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    }

    fn sanitize(reference: &str) -> String {
        reference
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .take(64)
            .collect()
    }
}

impl std::fmt::Debug for ManualProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManualProvider")
            .field("id", &self.id)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[async_trait]
impl PaymentProvider for ManualProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn create_session(
        &self,
        request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError> {
        if request.amount < 0 {
            return Err(PaymentError::InvalidRequest(
                "amount must not be negative".to_owned(),
            ));
        }
        let provider_session_id = format!(
            "{}_{}_{}",
            self.id,
            Self::sanitize(&request.reference),
            Self::random_hex()
        );
        Ok(ProviderSession {
            provider_session_id,
            status: PaymentStatus::Pending,
            data: serde_json::json!({ "provider": self.id }),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        if request.amount < 0 {
            return Err(PaymentError::InvalidRequest(
                "amount must not be negative".to_owned(),
            ));
        }
        if request.provider_session_id.is_empty() {
            return Err(PaymentError::InvalidRequest(
                "provider session id is required".to_owned(),
            ));
        }
        Ok(CaptureResult {
            provider_payment_id: format!("{}_{}", self.id, Self::random_hex()),
            amount: request.amount,
            currency_code: request.currency_code,
            status: PaymentStatus::Captured,
        })
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        if request.amount <= 0 {
            return Err(PaymentError::InvalidRequest(
                "refund amount must be positive".to_owned(),
            ));
        }
        if request.provider_payment_id.is_empty() {
            return Err(PaymentError::InvalidRequest(
                "provider payment id is required".to_owned(),
            ));
        }
        Ok(RefundResult {
            provider_refund_id: format!("{}_{}", self.id, Self::random_hex()),
            amount: request.amount,
            currency_code: request.currency_code,
            status: RefundStatus::Succeeded,
        })
    }

    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        let Some(secret) = self.secret.as_deref() else {
            return Err(PaymentError::InvalidSignature);
        };
        let signature = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(MANUAL_SIGNATURE_HEADER))
            .map(|(_, value)| value.as_str())
            .ok_or(PaymentError::InvalidSignature)?;
        let signature_bytes = hex::decode(signature).map_err(|_| PaymentError::InvalidSignature)?;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|_| PaymentError::InvalidSignature)?;
        mac.update(body);
        mac.verify_slice(&signature_bytes)
            .map_err(|_| PaymentError::InvalidSignature)?;

        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|_| PaymentError::InvalidRequest("invalid webhook payload".to_owned()))?;
        let event_id = payload
            .get("id")
            .or_else(|| payload.get("event_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PaymentError::InvalidRequest("webhook payload is missing an event id".to_owned())
            })?
            .to_owned();
        let event_type = payload
            .get("type")
            .or_else(|| payload.get("event"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PaymentError::InvalidRequest("webhook payload is missing an event type".to_owned())
            })?
            .to_owned();
        let data = payload
            .get("data")
            .cloned()
            .unwrap_or_else(|| payload.clone());
        Ok(VerifiedWebhook {
            event_id,
            event_type,
            data,
        })
    }
}

/// Thread-safe registry of available payment providers.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Arc<RwLock<HashMap<String, Arc<dyn PaymentProvider>>>>,
}

impl ProviderRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a provider, failing when its identifier is already present.
    pub fn register(&self, provider: Arc<dyn PaymentProvider>) -> Result<(), AppError> {
        let id = provider.id().to_owned();
        let mut providers = self
            .providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if providers.contains_key(&id) {
            return Err(AppError::conflict(format!(
                "payment provider already registered: {id}"
            )));
        }
        providers.insert(id, provider);
        Ok(())
    }

    /// Inserts or replaces a provider, returning the previous implementation.
    pub fn replace(&self, provider: Arc<dyn PaymentProvider>) -> Option<Arc<dyn PaymentProvider>> {
        let id = provider.id().to_owned();
        self.providers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, provider)
    }

    /// Looks up a provider by identifier.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn PaymentProvider>> {
        self.providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Returns all registered providers ordered by identifier.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<dyn PaymentProvider>> {
        let mut providers: Vec<_> = self
            .providers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        providers.sort_by(|left, right| left.id().cmp(right.id()));
        providers
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use fastcart_kernel::Metadata;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    use super::{
        CaptureRequest, CreateProviderSession, MANUAL_SIGNATURE_HEADER, ManualProvider,
        PaymentError, PaymentProvider, ProviderRegistry, RefundRequest,
    };

    fn provider(secret: Option<&str>) -> ManualProvider {
        ManualProvider::new("manual", secret.map(str::to_owned))
    }

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn create_request() -> CreateProviderSession {
        CreateProviderSession {
            reference: "paysess_1".to_owned(),
            amount: 1500,
            currency_code: "USD".to_owned(),
            metadata: Metadata::empty(),
        }
    }

    #[tokio::test]
    async fn manual_provider_creates_unique_sessions() {
        let provider = provider(Some("secret"));
        let first = provider.create_session(create_request()).await.unwrap();
        let second = provider.create_session(create_request()).await.unwrap();

        assert_eq!(first.status, crate::domain::PaymentStatus::Pending);
        assert!(first.provider_session_id.starts_with("manual_paysess_1_"));
        assert_ne!(first.provider_session_id, second.provider_session_id);
    }

    #[tokio::test]
    async fn manual_provider_captures_and_refunds() {
        let provider = provider(Some("secret"));
        let captured = provider
            .capture(CaptureRequest {
                provider_session_id: "manual_session".to_owned(),
                amount: 1500,
                currency_code: "USD".to_owned(),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        assert_eq!(captured.status, crate::domain::PaymentStatus::Captured);
        assert_eq!(captured.amount, 1500);
        assert!(!captured.provider_payment_id.is_empty());

        let refund = provider
            .refund(RefundRequest {
                provider_payment_id: captured.provider_payment_id,
                amount: 500,
                currency_code: "USD".to_owned(),
                reason: Some("damaged".to_owned()),
                idempotency_key: Some("refund-1".to_owned()),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        assert_eq!(refund.status, crate::domain::RefundStatus::Succeeded);
        assert_eq!(refund.amount, 500);
        assert!(!refund.provider_refund_id.is_empty());
    }

    #[test]
    fn manual_provider_verifies_hmac_signatures() {
        let provider = provider(Some("top-secret"));
        let body =
            br#"{"id":"evt_1","type":"payment.captured","data":{"provider_payment_id":"pay_1"}}"#;
        let headers =
            BTreeMap::from([(MANUAL_SIGNATURE_HEADER.to_owned(), sign("top-secret", body))]);

        let event = provider.verify_webhook(&headers, body).unwrap();
        assert_eq!(event.event_id, "evt_1");
        assert_eq!(event.event_type, "payment.captured");
        assert_eq!(event.data["provider_payment_id"], "pay_1");
    }

    #[test]
    fn manual_provider_rejects_bad_or_missing_signatures() {
        let provider = provider(Some("top-secret"));
        let body = br#"{"id":"evt_1","type":"payment.captured"}"#;

        let bad_headers = BTreeMap::from([(
            MANUAL_SIGNATURE_HEADER.to_owned(),
            sign("wrong-secret", body),
        )]);
        assert_eq!(
            provider.verify_webhook(&bad_headers, body),
            Err(PaymentError::InvalidSignature)
        );
        assert_eq!(
            provider.verify_webhook(&BTreeMap::new(), body),
            Err(PaymentError::InvalidSignature)
        );
    }

    #[test]
    fn manual_provider_without_secret_rejects_webhooks() {
        let provider = ManualProvider::sandbox();
        let body = br#"{"id":"evt_1","type":"payment.captured"}"#;
        assert_eq!(
            provider.verify_webhook(&BTreeMap::new(), body),
            Err(PaymentError::InvalidSignature)
        );
    }

    #[test]
    fn registry_registers_replaces_gets_and_lists() {
        let registry = ProviderRegistry::new();
        let first: Arc<dyn PaymentProvider> = Arc::new(provider(None));
        registry.register(first.clone()).unwrap();
        assert!(registry.register(first).is_err());
        assert!(registry.get("manual").is_some());

        let replacement: Arc<dyn PaymentProvider> = Arc::new(provider(Some("secret")));
        assert!(registry.replace(replacement).is_some());
        assert_eq!(registry.list().len(), 1);
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn debug_output_never_contains_the_secret() {
        let rendered = format!("{:?}", provider(Some("super-secret")));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("super-secret"));
    }
}
