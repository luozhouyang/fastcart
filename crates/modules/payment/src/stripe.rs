//! Production payment provider backed by the Stripe Payment Intents API.
//!
//! Amounts are passed to Stripe exactly as supplied by the payment service.
//! The application stores amounts in the currency's minor unit, which matches
//! Stripe's `amount` semantics for two-decimal and zero-decimal currencies
//! (for example `1500` is `$15.00`/`¥1500`) as well as three-decimal
//! currencies.
//!
//! Provider secrets are never rendered in `Debug` output or included in
//! webhook payloads. The PaymentIntent client secret is the only secret-like
//! value exposed through [`crate::ProviderSession::data`], because a storefront
//! needs it to confirm the payment; the payment service keeps it out of the
//! persisted session record.

use std::collections::{BTreeMap, HashMap};

use async_trait::async_trait;
use fastcart_kernel::{Metadata, now_timestamp};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::domain::{PaymentStatus, RefundStatus};
use crate::provider::{
    CaptureRequest, CaptureResult, CreateProviderSession, PaymentError, PaymentProvider,
    ProviderSession, RefundRequest, RefundResult, VerifiedWebhook,
};

/// Stable provider identifier.
pub const STRIPE_PROVIDER_ID: &str = "stripe";

/// Header carrying a Stripe webhook signature.
pub const STRIPE_SIGNATURE_HEADER: &str = "stripe-signature";

/// Maximum accepted difference between a signature timestamp and now.
const SIGNATURE_TOLERANCE_SECONDS: u64 = 300;

/// Metadata keys forwarded to Stripe.
///
/// Only known identifiers are copied so provider data such as a client secret
/// can never leak into Stripe metadata on later capture or refund calls.
const METADATA_IDENTIFIER_KEYS: [&str; 4] = ["cart_id", "checkout_id", "order_id", "session_id"];

/// Maximum length of a Stripe metadata value.
const METADATA_VALUE_LIMIT: usize = 500;

type HmacSha256 = Hmac<Sha256>;

/// Stripe Payment Intents provider.
pub struct StripeProvider {
    client: stripe::Client,
    webhook_secret: Option<String>,
}

impl StripeProvider {
    /// Creates a provider with a Stripe secret key and an optional webhook
    /// signing secret.
    ///
    /// When no webhook secret is supplied every webhook delivery is rejected.
    #[must_use]
    pub fn new(secret_key: impl Into<String>, webhook_secret: Option<String>) -> Self {
        Self {
            client: stripe::Client::new(secret_key),
            webhook_secret: webhook_secret.filter(|secret| !secret.trim().is_empty()),
        }
    }

    /// Creates a provider around an existing client.
    ///
    /// This is primarily useful for tests that point the client at a local
    /// mock server instead of the Stripe API.
    #[must_use]
    pub fn from_client(client: stripe::Client, webhook_secret: Option<String>) -> Self {
        Self {
            client,
            webhook_secret: webhook_secret.filter(|secret| !secret.trim().is_empty()),
        }
    }

    fn client_with_key(&self, key: Option<&str>) -> stripe::Client {
        match key.map(str::trim).filter(|key| !key.is_empty()) {
            Some(key) => self
                .client
                .clone()
                .with_strategy(stripe::RequestStrategy::Idempotent(key.to_owned())),
            None => self.client.clone(),
        }
    }
}

impl std::fmt::Debug for StripeProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StripeProvider")
            .field("id", &STRIPE_PROVIDER_ID)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "<redacted>"),
            )
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PaymentProvider for StripeProvider {
    fn id(&self) -> &str {
        STRIPE_PROVIDER_ID
    }

    async fn create_session(
        &self,
        request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError> {
        if request.amount <= 0 {
            return Err(PaymentError::InvalidRequest(
                "Stripe payment amount must be positive".to_owned(),
            ));
        }
        if request.reference.trim().is_empty() {
            return Err(PaymentError::InvalidRequest(
                "payment reference is required".to_owned(),
            ));
        }
        let currency = parse_currency(&request.currency_code)?;
        let mut params = stripe::CreatePaymentIntent::new(request.amount, currency);
        params.automatic_payment_methods =
            Some(stripe::CreatePaymentIntentAutomaticPaymentMethods {
                allow_redirects: None,
                enabled: true,
            });
        // Capture explicitly from the application so the service can own the
        // transition from authorized to captured.
        params.capture_method = Some(stripe::PaymentIntentCaptureMethod::Manual);
        params.metadata = Some(build_metadata(
            &request.metadata,
            Some(request.reference.as_str()),
            None,
        ));

        let idempotency_key = format!("session:{}", request.reference);
        let client = self.client_with_key(Some(idempotency_key.as_str()));
        let intent = stripe::PaymentIntent::create(&client, params)
            .await
            .map_err(stripe_error)?;

        let data = json!({
            "provider": STRIPE_PROVIDER_ID,
            "payment_intent_id": intent.id.to_string(),
            "status": intent.status.as_str(),
            "client_secret": intent.client_secret,
        });
        Ok(ProviderSession {
            provider_session_id: intent.id.to_string(),
            status: map_payment_intent_status(intent.status),
            data,
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        if request.amount < 0 {
            return Err(PaymentError::InvalidRequest(
                "capture amount must not be negative".to_owned(),
            ));
        }
        if request.provider_session_id.trim().is_empty() {
            return Err(PaymentError::InvalidRequest(
                "provider session id is required".to_owned(),
            ));
        }
        let intent_id: stripe::PaymentIntentId =
            request.provider_session_id.parse().map_err(|_| {
                PaymentError::InvalidRequest("invalid Stripe payment intent id".to_owned())
            })?;
        // Read the current state first so retries after a successful capture
        // remain idempotent even when the caller did not persist the result.
        let intent = stripe::PaymentIntent::retrieve(&self.client, &intent_id, &[])
            .await
            .map_err(stripe_error)?;

        match intent.status {
            stripe::PaymentIntentStatus::Succeeded => Ok(capture_result(
                &intent,
                request.amount,
                request.currency_code,
            )),
            stripe::PaymentIntentStatus::RequiresCapture => {
                let params = stripe::CapturePaymentIntent {
                    amount_to_capture: u64::try_from(request.amount)
                        .ok()
                        .filter(|amount| *amount > 0),
                    application_fee_amount: None,
                };
                let client = self.client_with_key(request.idempotency_key.as_deref());
                let captured = stripe::PaymentIntent::capture(
                    &client,
                    request.provider_session_id.as_str(),
                    params,
                )
                .await
                .map_err(stripe_error)?;
                if captured.status != stripe::PaymentIntentStatus::Succeeded {
                    return Err(PaymentError::Provider(format!(
                        "Stripe did not capture the payment intent (status {})",
                        captured.status
                    )));
                }
                Ok(capture_result(
                    &captured,
                    request.amount,
                    request.currency_code,
                ))
            }
            stripe::PaymentIntentStatus::Canceled => Err(PaymentError::Provider(
                "payment intent was canceled".to_owned(),
            )),
            status => Err(PaymentError::InvalidRequest(format!(
                "payment intent is not capturable (status {status})"
            ))),
        }
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        if request.amount <= 0 {
            return Err(PaymentError::InvalidRequest(
                "refund amount must be positive".to_owned(),
            ));
        }
        if request.provider_payment_id.trim().is_empty() {
            return Err(PaymentError::InvalidRequest(
                "provider payment id is required".to_owned(),
            ));
        }
        let payment_intent_id: stripe::PaymentIntentId =
            request.provider_payment_id.parse().map_err(|_| {
                PaymentError::InvalidRequest("invalid Stripe payment intent id".to_owned())
            })?;
        let mut params = stripe::CreateRefund::new();
        params.payment_intent = Some(payment_intent_id);
        params.amount = Some(request.amount);
        params.metadata = Some(build_metadata(
            &request.metadata,
            None,
            request.reason.as_deref(),
        ));
        params.reason = stripe_refund_reason(request.reason.as_deref());

        let client = self.client_with_key(request.idempotency_key.as_deref());
        let refund = stripe::Refund::create(&client, params)
            .await
            .map_err(stripe_error)?;
        Ok(RefundResult {
            provider_refund_id: refund.id.to_string(),
            amount: refund.amount,
            currency_code: request.currency_code,
            status: map_refund_status(refund.status.as_deref()),
        })
    }

    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        let Some(secret) = self.webhook_secret.as_deref() else {
            return Err(PaymentError::InvalidSignature);
        };
        let signature = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(STRIPE_SIGNATURE_HEADER))
            .map(|(_, value)| value.as_str())
            .ok_or(PaymentError::InvalidSignature)?;
        let payload = std::str::from_utf8(body).map_err(|_| {
            PaymentError::InvalidRequest("webhook payload must be UTF-8".to_owned())
        })?;
        verify_signature(signature, payload, secret, now_timestamp())?;

        // Parse only after the signature has been verified.
        let document: Value = serde_json::from_str(payload)
            .map_err(|_| PaymentError::InvalidRequest("invalid webhook payload".to_owned()))?;
        let event_id = document
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PaymentError::InvalidRequest("webhook payload is missing an event id".to_owned())
            })?
            .to_owned();
        let raw_event_type = document
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                PaymentError::InvalidRequest("webhook payload is missing an event type".to_owned())
            })?;
        let object = document
            .pointer("/data/object")
            .cloned()
            .unwrap_or(Value::Null);
        let (event_type, data) = normalize_event(raw_event_type, &object);
        Ok(VerifiedWebhook {
            event_id,
            event_type,
            data,
        })
    }
}

/// Maps a Stripe PaymentIntent status onto the provider-independent model.
#[must_use]
pub fn map_payment_intent_status(status: stripe::PaymentIntentStatus) -> PaymentStatus {
    match status {
        stripe::PaymentIntentStatus::Succeeded => PaymentStatus::Captured,
        stripe::PaymentIntentStatus::RequiresCapture => PaymentStatus::Authorized,
        stripe::PaymentIntentStatus::RequiresAction => PaymentStatus::RequiresAction,
        stripe::PaymentIntentStatus::Canceled => PaymentStatus::Canceled,
        stripe::PaymentIntentStatus::Processing
        | stripe::PaymentIntentStatus::RequiresConfirmation
        | stripe::PaymentIntentStatus::RequiresPaymentMethod => PaymentStatus::Pending,
    }
}

/// Builds the `Stripe-Signature` header value for a payload.
///
/// This mirrors Stripe's HMAC-SHA256 scheme. It is exposed so tests and local
/// tooling can deliver signed webhooks without depending on Stripe's servers.
#[must_use]
pub fn signature_header(secret: &str, timestamp: i64, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    format!(
        "t={timestamp},v1={}",
        hex::encode(mac.finalize().into_bytes())
    )
}

fn verify_signature(
    header: &str,
    payload: &str,
    secret: &str,
    now: i64,
) -> Result<(), PaymentError> {
    let mut timestamp: Option<&str> = None;
    let mut signatures: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        match key.trim() {
            "t" => timestamp = Some(value.trim()),
            "v1" => signatures.push(value.trim()),
            _ => {}
        }
    }

    let timestamp = timestamp.ok_or(PaymentError::InvalidSignature)?;
    let seconds = timestamp
        .parse::<i64>()
        .map_err(|_| PaymentError::InvalidSignature)?;
    if now.abs_diff(seconds) > SIGNATURE_TOLERANCE_SECONDS {
        return Err(PaymentError::InvalidSignature);
    }

    let signed_payload = format!("{timestamp}.{payload}");
    for signature in signatures {
        let Ok(bytes) = hex::decode(signature) else {
            continue;
        };
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|_| PaymentError::InvalidSignature)?;
        mac.update(signed_payload.as_bytes());
        if mac.verify_slice(&bytes).is_ok() {
            return Ok(());
        }
    }
    Err(PaymentError::InvalidSignature)
}

fn normalize_event(raw_event_type: &str, object: &Value) -> (String, Value) {
    let (event_type, mut data) = match raw_event_type {
        "payment_intent.succeeded" => ("payment.succeeded", payment_intent_event_data(object)),
        "payment_intent.payment_failed" => ("payment.failed", payment_intent_event_data(object)),
        "payment_intent.canceled" => ("payment.canceled", payment_intent_event_data(object)),
        "charge.refunded" => ("refund.succeeded", charge_refunded_event_data(object)),
        other => (other, generic_event_data(object)),
    };
    if let Value::Object(map) = &mut data {
        map.insert(
            "provider".to_owned(),
            Value::String(STRIPE_PROVIDER_ID.to_owned()),
        );
        map.insert(
            "provider_event_type".to_owned(),
            Value::String(raw_event_type.to_owned()),
        );
    }
    (event_type.to_owned(), data)
}

fn payment_intent_event_data(object: &Value) -> Value {
    let id = object.get("id").and_then(Value::as_str);
    let amount = object
        .get("amount_received")
        .and_then(Value::as_i64)
        .filter(|amount| *amount > 0)
        .or_else(|| object.get("amount").and_then(Value::as_i64))
        .unwrap_or(0);
    let failure = object.get("last_payment_error");
    json!({
        "provider_payment_id": id,
        "provider_session_id": id,
        "status": object.get("status").and_then(Value::as_str),
        "amount": amount,
        "currency": object.get("currency").and_then(Value::as_str).map(str::to_ascii_uppercase),
        "metadata": identifier_metadata(object.get("metadata")),
        "failure_code": failure.and_then(|error| error.get("code")).and_then(Value::as_str),
        "failure_message": failure.and_then(|error| error.get("message")).and_then(Value::as_str),
    })
}

fn charge_refunded_event_data(object: &Value) -> Value {
    let payment_intent_id = object_id(object.get("payment_intent"));
    let latest_refund = object
        .get("refunds")
        .and_then(|refunds| refunds.get("data"))
        .and_then(Value::as_array)
        .and_then(|refunds| {
            refunds
                .iter()
                .max_by_key(|refund| refund.get("created").and_then(Value::as_i64).unwrap_or(0))
        });
    let provider_refund_id = latest_refund
        .and_then(|refund| refund.get("id"))
        .and_then(Value::as_str);
    let amount = latest_refund
        .and_then(|refund| refund.get("amount"))
        .and_then(Value::as_i64)
        .or_else(|| object.get("amount_refunded").and_then(Value::as_i64));
    json!({
        "provider_payment_id": payment_intent_id,
        "provider_refund_id": provider_refund_id,
        "provider_charge_id": object.get("id").and_then(Value::as_str),
        "status": "succeeded",
        "amount": amount,
        "currency": object.get("currency").and_then(Value::as_str).map(str::to_ascii_uppercase),
        "refunded": object.get("refunded").and_then(Value::as_bool),
    })
}

fn generic_event_data(object: &Value) -> Value {
    json!({
        "object_id": object.get("id").and_then(Value::as_str),
        "object_type": object.get("object").and_then(Value::as_str),
        "status": object.get("status").and_then(Value::as_str),
    })
}

fn object_id(value: Option<&Value>) -> Option<&str> {
    match value? {
        Value::String(value) if !value.is_empty() => Some(value.as_str()),
        Value::Object(map) => map.get("id").and_then(Value::as_str),
        _ => None,
    }
}

fn identifier_metadata(value: Option<&Value>) -> Value {
    let mut metadata = serde_json::Map::new();
    if let Some(value) = value.and_then(Value::as_object) {
        for key in METADATA_IDENTIFIER_KEYS {
            if let Some(entry) = value.get(key) {
                metadata.insert(key.to_owned(), entry.clone());
            }
        }
    }
    Value::Object(metadata)
}

fn build_metadata(
    metadata: &Metadata,
    reference: Option<&str>,
    reason: Option<&str>,
) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for key in METADATA_IDENTIFIER_KEYS {
        if let Some(value) = metadata.get(key).and_then(stripe_scalar_value) {
            values.insert(key.to_owned(), truncate_metadata_value(value));
        }
    }
    if let Some(reference) = reference.filter(|value| !value.trim().is_empty()) {
        values
            .entry("session_id".to_owned())
            .or_insert_with(|| truncate_metadata_value(reference.to_owned()));
    }
    if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
        values
            .entry("refund_reason".to_owned())
            .or_insert_with(|| truncate_metadata_value(reason.to_owned()));
    }
    values
}

fn stripe_scalar_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn truncate_metadata_value(value: String) -> String {
    value.chars().take(METADATA_VALUE_LIMIT).collect()
}

fn parse_currency(currency_code: &str) -> Result<stripe::Currency, PaymentError> {
    currency_code
        .trim()
        .to_ascii_lowercase()
        .parse::<stripe::Currency>()
        .map_err(|_| {
            PaymentError::InvalidRequest(format!(
                "Stripe does not support currency {currency_code}"
            ))
        })
}

fn stripe_refund_reason(reason: Option<&str>) -> Option<stripe::RefundReasonFilter> {
    match reason?.trim().to_ascii_lowercase().as_str() {
        "duplicate" => Some(stripe::RefundReasonFilter::Duplicate),
        "fraudulent" => Some(stripe::RefundReasonFilter::Fraudulent),
        "requested_by_customer" => Some(stripe::RefundReasonFilter::RequestedByCustomer),
        _ => None,
    }
}

fn map_refund_status(status: Option<&str>) -> RefundStatus {
    match status {
        Some("succeeded") => RefundStatus::Succeeded,
        Some("failed" | "canceled" | "cancelled") => RefundStatus::Failed,
        _ => RefundStatus::Pending,
    }
}

fn capture_result(
    intent: &stripe::PaymentIntent,
    requested_amount: i64,
    currency_code: String,
) -> CaptureResult {
    CaptureResult {
        provider_payment_id: intent.id.to_string(),
        amount: if intent.amount_received > 0 {
            intent.amount_received
        } else {
            requested_amount
        },
        currency_code,
        status: PaymentStatus::Captured,
    }
}

fn stripe_error(error: stripe::StripeError) -> PaymentError {
    // Stripe errors contain provider messages only; request secrets are never
    // included in the renderable error.
    PaymentError::Provider(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fastcart_kernel::{Metadata, now_timestamp};

    use super::{
        STRIPE_SIGNATURE_HEADER, StripeProvider, build_metadata, map_payment_intent_status,
        signature_header,
    };
    use crate::domain::PaymentStatus;
    use crate::provider::{PaymentError, PaymentProvider};

    const SECRET: &str = "whsec_test_secret";
    const CLIENT_SECRET: &str = "pi_test_1_client_secret";

    fn provider(secret: Option<&str>) -> StripeProvider {
        StripeProvider::new("sk_test_dummy", secret.map(str::to_owned))
    }

    fn signed_headers(secret: &str, timestamp: i64, payload: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(
            STRIPE_SIGNATURE_HEADER.to_owned(),
            signature_header(secret, timestamp, payload),
        )])
    }

    fn payment_intent_event(event_type: &str, status: &str) -> String {
        format!(
            r#"{{"id":"evt_test_1","object":"event","type":"{event_type}","data":{{"object":{{"id":"pi_test_1","object":"payment_intent","status":"{status}","amount":1500,"amount_received":1500,"currency":"usd","client_secret":"{CLIENT_SECRET}","metadata":{{"session_id":"sess_1","checkout_id":"chk_1"}}}}}}}}"#
        )
    }

    #[test]
    fn maps_every_payment_intent_status() {
        for (status, expected) in [
            (
                stripe::PaymentIntentStatus::Succeeded,
                PaymentStatus::Captured,
            ),
            (
                stripe::PaymentIntentStatus::RequiresCapture,
                PaymentStatus::Authorized,
            ),
            (
                stripe::PaymentIntentStatus::RequiresAction,
                PaymentStatus::RequiresAction,
            ),
            (
                stripe::PaymentIntentStatus::Canceled,
                PaymentStatus::Canceled,
            ),
            (
                stripe::PaymentIntentStatus::Processing,
                PaymentStatus::Pending,
            ),
            (
                stripe::PaymentIntentStatus::RequiresConfirmation,
                PaymentStatus::Pending,
            ),
            (
                stripe::PaymentIntentStatus::RequiresPaymentMethod,
                PaymentStatus::Pending,
            ),
        ] {
            assert_eq!(map_payment_intent_status(status), expected, "{status}");
        }
    }

    #[test]
    fn accepts_a_valid_signature_and_normalizes_success() {
        let provider = provider(Some(SECRET));
        let payload = payment_intent_event("payment_intent.succeeded", "succeeded");
        let headers = signed_headers(SECRET, now_timestamp(), &payload);

        let event = provider
            .verify_webhook(&headers, payload.as_bytes())
            .unwrap();
        assert_eq!(event.event_id, "evt_test_1");
        assert_eq!(event.event_type, "payment.succeeded");
        assert_eq!(event.data["provider"], "stripe");
        assert_eq!(
            event.data["provider_event_type"],
            "payment_intent.succeeded"
        );
        assert_eq!(event.data["provider_payment_id"], "pi_test_1");
        assert_eq!(event.data["provider_session_id"], "pi_test_1");
        assert_eq!(event.data["amount"], 1500);
        assert_eq!(event.data["currency"], "USD");
        assert_eq!(event.data["metadata"]["checkout_id"], "chk_1");
        // The client secret must never be copied into verified webhook data.
        assert!(!event.data.to_string().contains(CLIENT_SECRET));
    }

    #[test]
    fn normalizes_failure_and_cancellation() {
        let provider = provider(Some(SECRET));

        let failed =
            payment_intent_event("payment_intent.payment_failed", "requires_payment_method");
        let headers = signed_headers(SECRET, now_timestamp(), &failed);
        let event = provider
            .verify_webhook(&headers, failed.as_bytes())
            .unwrap();
        assert_eq!(event.event_type, "payment.failed");
        assert_eq!(event.data["provider_payment_id"], "pi_test_1");

        let canceled = payment_intent_event("payment_intent.canceled", "canceled");
        let headers = signed_headers(SECRET, now_timestamp(), &canceled);
        let event = provider
            .verify_webhook(&headers, canceled.as_bytes())
            .unwrap();
        assert_eq!(event.event_type, "payment.canceled");
        assert_eq!(event.data["provider_event_type"], "payment_intent.canceled");
    }

    #[test]
    fn normalizes_charge_refunded_to_the_latest_refund() {
        let provider = provider(Some(SECRET));
        let payload = r#"{
            "id": "evt_refund_1",
            "object": "event",
            "type": "charge.refunded",
            "data": {
                "object": {
                    "id": "ch_test_1",
                    "object": "charge",
                    "payment_intent": "pi_test_1",
                    "currency": "usd",
                    "amount_refunded": 900,
                    "refunded": false,
                    "refunds": {
                        "data": [
                            {"id": "re_old", "object": "refund", "amount": 400, "created": 10},
                            {"id": "re_new", "object": "refund", "amount": 500, "created": 20}
                        ]
                    }
                }
            }
        }"#;
        let headers = signed_headers(SECRET, now_timestamp(), payload);

        let event = provider
            .verify_webhook(&headers, payload.as_bytes())
            .unwrap();
        assert_eq!(event.event_type, "refund.succeeded");
        assert_eq!(event.data["provider_payment_id"], "pi_test_1");
        assert_eq!(event.data["provider_refund_id"], "re_new");
        assert_eq!(event.data["amount"], 500);
        assert_eq!(event.data["currency"], "USD");
    }

    #[test]
    fn stores_unknown_events_without_exposing_the_raw_payload() {
        let provider = provider(Some(SECRET));
        let payload = r#"{
            "id": "evt_unknown_1",
            "object": "event",
            "type": "customer.created",
            "data": {"object": {"id": "cus_1", "object": "customer", "email": "buyer@example.com"}}
        }"#;
        let headers = signed_headers(SECRET, now_timestamp(), payload);

        let event = provider
            .verify_webhook(&headers, payload.as_bytes())
            .unwrap();
        assert_eq!(event.event_type, "customer.created");
        assert_eq!(event.data["object_id"], "cus_1");
        assert!(event.data.get("email").is_none());
    }

    #[test]
    fn rejects_bad_missing_and_stale_signatures() {
        let provider = provider(Some(SECRET));
        let payload = payment_intent_event("payment_intent.succeeded", "succeeded");

        let bad = signed_headers("wrong-secret", now_timestamp(), &payload);
        assert_eq!(
            provider.verify_webhook(&bad, payload.as_bytes()),
            Err(PaymentError::InvalidSignature)
        );

        assert_eq!(
            provider.verify_webhook(&BTreeMap::new(), payload.as_bytes()),
            Err(PaymentError::InvalidSignature)
        );

        let stale = signed_headers(SECRET, now_timestamp() - 301, &payload);
        assert_eq!(
            provider.verify_webhook(&stale, payload.as_bytes()),
            Err(PaymentError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_webhooks_when_no_secret_is_configured() {
        let provider = provider(None);
        let payload = payment_intent_event("payment_intent.succeeded", "succeeded");
        let headers = signed_headers(SECRET, now_timestamp(), &payload);
        assert_eq!(
            provider.verify_webhook(&headers, payload.as_bytes()),
            Err(PaymentError::InvalidSignature)
        );
    }

    #[test]
    fn metadata_copy_is_limited_to_identifiers() {
        let mut metadata = Metadata::empty();
        metadata.insert("cart_id", serde_json::json!("cart_1"));
        metadata.insert("order_id", serde_json::json!(42));
        metadata.insert("client_secret", serde_json::json!("must-not-leak"));
        metadata.insert("nested", serde_json::json!({"a": 1}));

        let values = build_metadata(&metadata, Some("sess_1"), Some("damaged"));
        assert_eq!(values.get("cart_id").map(String::as_str), Some("cart_1"));
        assert_eq!(values.get("order_id").map(String::as_str), Some("42"));
        assert_eq!(values.get("session_id").map(String::as_str), Some("sess_1"));
        assert_eq!(
            values.get("refund_reason").map(String::as_str),
            Some("damaged")
        );
        assert!(!values.contains_key("client_secret"));
        assert!(!values.contains_key("nested"));
    }

    #[test]
    fn debug_output_never_contains_the_webhook_secret() {
        let rendered = format!("{:?}", provider(Some("whsec_super_secret")));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("whsec_super_secret"));
    }
}
