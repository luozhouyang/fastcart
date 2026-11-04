//! Stripe webhook end-to-end coverage: provider gating, signature verification
//! and idempotent payment-session updates over HTTP.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreId, now_timestamp};
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession, PaymentError,
    PaymentProvider, PaymentService, PaymentStatus, ProviderSession, RefundRequest, RefundResult,
    StripeProvider, VerifiedWebhook, signature_header,
};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const DOMAIN: &str = "shop.example.com";
const STRIPE_SECRET_KEY: &str = "sk_test_dummy";
const STRIPE_WEBHOOK_SECRET: &str = "whsec_test_secret";
const PROVIDER_SESSION_ID: &str = "pi_test_123";
const CLIENT_SECRET: &str = "pi_test_1_client_secret";

struct Fixture {
    app: Router,
    context: Arc<AppContext>,
    database: StoreDatabase,
    session_id: String,
    _dir: tempfile::TempDir,
}

/// Returns a deterministic provider session without touching Stripe.
struct StubProvider;

#[async_trait]
impl PaymentProvider for StubProvider {
    fn id(&self) -> &str {
        "stripe"
    }

    async fn create_session(
        &self,
        _request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError> {
        Ok(ProviderSession {
            provider_session_id: PROVIDER_SESSION_ID.to_owned(),
            status: PaymentStatus::Pending,
            data: json!({
                "provider": "stripe",
                "payment_intent_id": PROVIDER_SESSION_ID,
                "status": "requires_payment_method",
                "client_secret": CLIENT_SECRET,
            }),
        })
    }

    async fn capture(&self, _request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        Err(PaymentError::Provider("not used by this test".to_owned()))
    }

    async fn refund(&self, _request: RefundRequest) -> Result<RefundResult, PaymentError> {
        Err(PaymentError::Provider("not used by this test".to_owned()))
    }

    fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        _body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        Err(PaymentError::InvalidSignature)
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let context = Arc::new(AppContext::new(
        ApiConfig {
            cookie_secure: false,
            backup_dir: dir.path().join("backups"),
            stripe_secret_key: Some(STRIPE_SECRET_KEY.to_owned()),
            stripe_webhook_secret: Some(STRIPE_WEBHOOK_SECRET.to_owned()),
            ..ApiConfig::default()
        },
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    assert!(
        context.payment_providers.get("stripe").is_some(),
        "Stripe provider should be registered when both credentials are set"
    );
    let app = build_router(context.clone());

    let (status, _, cookies) = send(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, store, _) = send(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": DOMAIN
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = store["id"].as_str().unwrap().to_owned();
    let database = context
        .databases
        .database(&StoreId::from_string(store_id))
        .await
        .unwrap();

    // Create a local Stripe session without a network call, then restore the
    // real provider so the webhook route verifies against Stripe's scheme.
    let real_provider = context
        .payment_providers
        .replace(Arc::new(StubProvider))
        .expect("Stripe provider is registered");
    let payments =
        PaymentService::with_registry(database.clone(), context.payment_providers.clone());
    let payment_session = payments
        .create_session(CreatePaymentSessionInput {
            metadata: Metadata::empty(),
            ..CreatePaymentSessionInput::new("stripe", 1_500, "USD")
        })
        .await
        .unwrap();
    assert_eq!(
        payment_session.provider_session_id.as_deref(),
        Some(PROVIDER_SESSION_ID)
    );
    assert_eq!(
        payment_session.data.get("client_secret"),
        Some(&json!(CLIENT_SECRET)),
        "the caller receives the client secret"
    );
    let stored = payments
        .get_session(payment_session.id.as_str())
        .await
        .unwrap();
    assert!(
        stored.data.get("client_secret").is_none(),
        "the client secret must not be persisted"
    );
    context
        .payment_providers
        .replace(real_provider)
        .expect("the stub provider was registered");

    Fixture {
        app,
        context,
        database,
        session_id: payment_session.id.as_str().to_owned(),
        _dir: dir,
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", DOMAIN);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value, cookies)
}

async fn send_webhook(app: &Router, payload: &str, signature: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/payments/stripe")
        .header("host", DOMAIN)
        .header("content-type", "application/json");
    if let Some(signature) = signature {
        builder = builder.header("stripe-signature", signature);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(payload.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

fn payment_intent_succeeded(event_id: &str) -> String {
    json!({
        "id": event_id,
        "object": "event",
        "type": "payment_intent.succeeded",
        "data": {
            "object": {
                "id": PROVIDER_SESSION_ID,
                "object": "payment_intent",
                "status": "succeeded",
                "amount": 1_500,
                "amount_received": 1_500,
                "currency": "usd",
                "client_secret": CLIENT_SECRET,
                "metadata": {"session_id": "local_session"}
            }
        }
    })
    .to_string()
}

#[tokio::test]
async fn stripe_is_not_registered_without_both_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(1),
    ));

    let without_credentials = Arc::new(AppContext::new(
        ApiConfig {
            backup_dir: dir.path().join("backups"),
            ..ApiConfig::default()
        },
        platform.clone(),
        databases.clone(),
        StoreCache::default_capacity(),
    ));
    assert!(
        without_credentials
            .payment_providers
            .get("stripe")
            .is_none()
    );
    assert!(
        without_credentials
            .payment_providers
            .get("manual")
            .is_some()
    );

    let secret_key_only = Arc::new(AppContext::new(
        ApiConfig {
            backup_dir: dir.path().join("backups"),
            stripe_secret_key: Some(STRIPE_SECRET_KEY.to_owned()),
            ..ApiConfig::default()
        },
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    assert!(secret_key_only.payment_providers.get("stripe").is_none());

    let provider = StripeProvider::new(STRIPE_SECRET_KEY, Some(STRIPE_WEBHOOK_SECRET.to_owned()));
    assert_eq!(provider.id(), "stripe");
}

#[tokio::test]
async fn signed_payment_intent_succeeded_updates_the_session() {
    let fixture = fixture().await;
    let payload = payment_intent_succeeded("evt_succeeded_1");
    let signature = signature_header(STRIPE_WEBHOOK_SECRET, now_timestamp(), &payload);

    let (status, record) = send_webhook(&fixture.app, &payload, Some(&signature)).await;
    assert_eq!(status, StatusCode::OK, "{record}");
    assert_eq!(record["status"], "processed");
    assert_eq!(record["event_type"], "payment.succeeded");
    assert_eq!(record["attempts"], 1);

    let payments = PaymentService::with_registry(
        fixture.database.clone(),
        fixture.context.payment_providers.clone(),
    );
    let session = payments.get_session(&fixture.session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::Captured);
    let webhooks = payments
        .list_webhooks(None, fastcart_kernel::Pagination::default())
        .await
        .unwrap();
    assert_eq!(webhooks.count, 1);
    assert!(!webhooks.data[0].payload.contains(CLIENT_SECRET));
}

#[tokio::test]
async fn duplicate_delivery_is_idempotent() {
    let fixture = fixture().await;
    let payload = payment_intent_succeeded("evt_duplicate_1");
    let signature = signature_header(STRIPE_WEBHOOK_SECRET, now_timestamp(), &payload);

    let (first_status, first) = send_webhook(&fixture.app, &payload, Some(&signature)).await;
    let (second_status, second) = send_webhook(&fixture.app, &payload, Some(&signature)).await;
    assert_eq!(first_status, StatusCode::OK);
    assert_eq!(second_status, StatusCode::OK);
    assert_eq!(first["id"], second["id"]);
    assert_eq!(second["attempts"], 1);

    let payments = PaymentService::with_registry(
        fixture.database.clone(),
        fixture.context.payment_providers.clone(),
    );
    let webhooks = payments
        .list_webhooks(None, fastcart_kernel::Pagination::default())
        .await
        .unwrap();
    assert_eq!(webhooks.count, 1);
}

#[tokio::test]
async fn invalid_signature_is_rejected_without_persisting_the_event() {
    let fixture = fixture().await;
    let payload = payment_intent_succeeded("evt_bad_signature");
    let signature = signature_header("wrong-secret", now_timestamp(), &payload);

    let (status, body) = send_webhook(&fixture.app, &payload, Some(&signature)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, _) = send_webhook(&fixture.app, &payload, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let payments = PaymentService::with_registry(
        fixture.database.clone(),
        fixture.context.payment_providers.clone(),
    );
    let webhooks = payments
        .list_webhooks(None, fastcart_kernel::Pagination::default())
        .await
        .unwrap();
    assert_eq!(webhooks.count, 0);
    let session = payments.get_session(&fixture.session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::Pending);
}

#[tokio::test]
async fn canceled_payment_intent_marks_the_session_canceled() {
    let fixture = fixture().await;
    let payload = json!({
        "id": "evt_canceled_1",
        "object": "event",
        "type": "payment_intent.canceled",
        "data": {
            "object": {
                "id": PROVIDER_SESSION_ID,
                "object": "payment_intent",
                "status": "canceled",
                "amount": 1_500,
                "currency": "usd",
                "cancellation_reason": "abandoned"
            }
        }
    })
    .to_string();
    let signature = signature_header(STRIPE_WEBHOOK_SECRET, now_timestamp(), &payload);

    let (status, record) = send_webhook(&fixture.app, &payload, Some(&signature)).await;
    assert_eq!(status, StatusCode::OK, "{record}");
    assert_eq!(record["event_type"], "payment.canceled");

    let payments = PaymentService::with_registry(
        fixture.database.clone(),
        fixture.context.payment_providers.clone(),
    );
    let session = payments.get_session(&fixture.session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::Canceled);
}
