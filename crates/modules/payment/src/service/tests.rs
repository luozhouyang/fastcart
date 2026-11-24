use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, Metadata, OrderId, PageResult, Pagination, StoreId, now_timestamp,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::PaymentService;
use crate::domain::{
    CreatePaymentSessionInput, ListPaymentsFilter, PaymentStatus, RefundStatus, WebhookStatus,
};
use crate::provider::{
    CaptureRequest, CaptureResult, CreateProviderSession, MANUAL_SIGNATURE_HEADER, ManualProvider,
    PaymentError, PaymentProvider, ProviderSession, RefundRequest, RefundResult, VerifiedWebhook,
};

async fn setup() -> (PaymentService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PaymentService::new(db.clone());
    (service, db)
}

fn manual(secret: Option<&str>) -> Arc<dyn PaymentProvider> {
    Arc::new(ManualProvider::new("manual", secret.map(str::to_owned)))
}

fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

fn headers(secret: &str, body: &[u8]) -> BTreeMap<String, String> {
    BTreeMap::from([(MANUAL_SIGNATURE_HEADER.to_owned(), sign(secret, body))])
}

fn input(provider: &str, amount: i64) -> CreatePaymentSessionInput {
    CreatePaymentSessionInput::new(provider, amount, "usd")
}

async fn create_and_capture(
    service: &PaymentService,
    amount: i64,
) -> (String, crate::domain::Payment) {
    let session = service
        .create_session(input("manual", amount))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();
    (session.id.to_string(), payment)
}

struct TestProvider {
    id: String,
    fail_create: bool,
    fail_capture: bool,
    invalid_capture: bool,
    capture_failures_remaining: AtomicUsize,
    fail_refund: bool,
    invalid_refund: bool,
    create_calls: AtomicUsize,
    capture_calls: AtomicUsize,
    refund_calls: AtomicUsize,
}

impl TestProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            fail_create: false,
            fail_capture: false,
            invalid_capture: false,
            capture_failures_remaining: AtomicUsize::new(0),
            fail_refund: false,
            invalid_refund: false,
            create_calls: AtomicUsize::new(0),
            capture_calls: AtomicUsize::new(0),
            refund_calls: AtomicUsize::new(0),
        }
    }

    fn transient_capture(id: &str) -> Self {
        Self {
            capture_failures_remaining: AtomicUsize::new(1),
            ..Self::new(id)
        }
    }

    fn invalid_capture(id: &str) -> Self {
        Self {
            invalid_capture: true,
            ..Self::new(id)
        }
    }

    fn failing_refund(id: &str) -> Self {
        Self {
            fail_refund: true,
            ..Self::new(id)
        }
    }

    fn invalid_refund(id: &str) -> Self {
        Self {
            invalid_refund: true,
            ..Self::new(id)
        }
    }

    fn failing_create(id: &str) -> Self {
        Self {
            fail_create: true,
            ..Self::new(id)
        }
    }

    fn capture_count(&self) -> usize {
        self.capture_calls.load(Ordering::SeqCst)
    }

    fn refund_count(&self) -> usize {
        self.refund_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PaymentProvider for TestProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn create_session(
        &self,
        _request: CreateProviderSession,
    ) -> Result<ProviderSession, PaymentError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_create {
            return Err(PaymentError::Provider("create failed".to_owned()));
        }
        Ok(ProviderSession {
            provider_session_id: format!("{}_session", self.id),
            status: PaymentStatus::Pending,
            data: serde_json::json!({}),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        self.capture_calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid_capture {
            return Err(PaymentError::InvalidRequest(
                "capture request is invalid".to_owned(),
            ));
        }
        if self.fail_capture {
            return Err(PaymentError::Provider("capture failed".to_owned()));
        }
        let remaining = self.capture_failures_remaining.load(Ordering::SeqCst);
        if remaining > 0 {
            self.capture_failures_remaining
                .store(remaining - 1, Ordering::SeqCst);
            return Err(PaymentError::Provider("capture transport error".to_owned()));
        }
        Ok(CaptureResult {
            provider_payment_id: format!("{}_payment", self.id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: PaymentStatus::Captured,
        })
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        self.refund_calls.fetch_add(1, Ordering::SeqCst);
        if self.invalid_refund {
            return Err(PaymentError::InvalidRequest(
                "refund request is invalid".to_owned(),
            ));
        }
        if self.fail_refund {
            return Err(PaymentError::Provider("refund failed".to_owned()));
        }
        Ok(RefundResult {
            provider_refund_id: format!("{}_refund", self.id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: RefundStatus::Succeeded,
        })
    }

    fn verify_webhook(
        &self,
        _headers: &BTreeMap<String, String>,
        _body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        Err(PaymentError::InvalidSignature)
    }
}

#[tokio::test]
async fn creates_session_and_attempt() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut input = input("manual", 2500);
    input.metadata.insert("checkout", serde_json::json!("v1"));
    input.cart_id = Some(fastcart_kernel::CartId::new());

    let session = service.create_session(input).await.unwrap();
    assert_eq!(session.amount, 2500);
    assert_eq!(session.currency_code, "USD");
    assert_eq!(session.status, PaymentStatus::Pending);
    assert!(session.provider_session_id.is_some());
    assert_eq!(session.data.get("checkout"), Some(&serde_json::json!("v1")));

    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored, session);
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, PaymentStatus::Pending);
    assert_eq!(attempts[0].amount, 2500);
}

#[tokio::test]
async fn rejects_unknown_provider_negative_amount_and_bad_currency() {
    let (service, _db) = setup().await;

    let missing = service
        .create_session(input("missing", 100))
        .await
        .unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)));

    service.register_provider(manual(Some("secret"))).unwrap();
    let negative = service
        .create_session(input("manual", -1))
        .await
        .unwrap_err();
    assert!(matches!(negative, AppError::Validation(_)));

    let bad_currency = service
        .create_session(CreatePaymentSessionInput::new("manual", 100, "us"))
        .await
        .unwrap_err();
    assert!(matches!(bad_currency, AppError::Validation(_)));
}

#[tokio::test]
async fn full_flow_captures_and_refunds() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();

    let (session_id, payment) = create_and_capture(&service, 1000).await;
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(payment.amount, 1000);
    assert!(payment.provider_payment_id.is_some());
    assert!(payment.captured_at.is_some());

    let partial = service
        .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
        .await
        .unwrap();
    assert_eq!(partial.status, RefundStatus::Succeeded);
    assert_eq!(partial.amount, 400);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );

    let remainder = service
        .refund(payment.id.as_str(), 600, None)
        .await
        .unwrap();
    assert_eq!(remainder.amount, 600);
    let refunded = service.get_payment(payment.id.as_str()).await.unwrap();
    assert_eq!(refunded.status, PaymentStatus::Refunded);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        2
    );

    let captured_again = service.capture(&session_id).await.unwrap();
    assert_eq!(captured_again.id, payment.id);
    assert_eq!(captured_again.status, PaymentStatus::Refunded);
}

#[tokio::test]
async fn capture_is_idempotent_and_does_not_repeat_provider_call() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("counter"));
    service.register_provider(provider.clone()).unwrap();

    let session = service
        .create_session(input("counter", 1500))
        .await
        .unwrap();
    let first = service.capture(session.id.as_str()).await.unwrap();
    let second = service.capture(session.id.as_str()).await.unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(provider.capture_count(), 1);
}

#[tokio::test]
async fn rejects_over_refund_before_calling_provider() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("limited"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("limited", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    service
        .refund(payment.id.as_str(), 700, None)
        .await
        .unwrap();
    let over = service
        .refund(payment.id.as_str(), 400, None)
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)));
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );

    let zero = service
        .refund(payment.id.as_str(), 0, None)
        .await
        .unwrap_err();
    assert!(matches!(zero, AppError::Validation(_)));
}

#[tokio::test]
async fn rejects_refund_before_capture() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();

    let error = service
        .refund(session.id.as_str(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn provider_failures_surface_as_external_service_errors() {
    let (service, db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::failing_create("broken")))
        .unwrap();
    let error = service
        .create_session(input("broken", 100))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_sessions")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn concurrent_captures_never_call_the_provider_twice() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("manual"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let service = Arc::new(service);
    let first = service.clone();
    let second = service.clone();
    let id = session.id.to_string();
    let (left, right) = tokio::join!(first.capture(&id), second.capture(&id));
    assert!(left.is_ok() || right.is_ok());
    assert_eq!(provider.capture_count(), 1);
    let payments = service
        .list_payments(ListPaymentsFilter {
            order_id: None,
            status: None,
            pagination: Pagination::default(),
        })
        .await
        .unwrap();
    assert_eq!(payments.count, 1);
}

#[tokio::test]
async fn ambiguous_capture_failure_stays_retryable_and_retries_provider() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::transient_capture("flaky"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("flaky", 1000)).await.unwrap();

    let error = service.capture(session.id.as_str()).await.unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
    assert!(stored.data.get("capture_claim").is_none());
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].status, PaymentStatus::Failed);
    assert!(attempts[1].error.is_some());

    let payment = service.capture(session.id.as_str()).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(provider.capture_count(), 2);
    let payments = service
        .list_payments(ListPaymentsFilter {
            order_id: None,
            status: None,
            pagination: Pagination::default(),
        })
        .await
        .unwrap();
    assert_eq!(payments.count, 1);
}

#[tokio::test]
async fn invalid_capture_request_marks_session_and_attempt_failed() {
    let (service, _db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::invalid_capture("broken")))
        .unwrap();
    let session = service.create_session(input("broken", 1000)).await.unwrap();

    let error = service.capture(session.id.as_str()).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Failed);
    let attempts = service.list_attempts(session.id.as_str()).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].status, PaymentStatus::Failed);
    assert!(attempts[1].error.is_some());
}

#[tokio::test]
async fn definitive_refund_rejection_marks_the_refund_failed() {
    let (service, _db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::invalid_refund("broken")))
        .unwrap();
    let session = service.create_session(input("broken", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let error = service
        .refund(payment.id.as_str(), 100, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Failed);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn ambiguous_refund_failure_stays_pending_and_consumes_the_cap() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::failing_refund("unknown"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("unknown", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let error = service
        .refund(payment.id.as_str(), 700, Some("first".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Pending);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );

    // 400 more would fit only if the unknown 700 were excluded from the
    // cap. Pending refunds count, so this must be rejected.
    let over = service
        .refund(payment.id.as_str(), 400, Some("second".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)));
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn duplicate_refund_key_returns_the_existing_refund() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("dedupe", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:dedupe".to_owned()),
        )
        .await
        .unwrap();
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:dedupe".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn keyless_refund_retry_reuses_the_same_refund() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("keyless"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("keyless", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund(payment.id.as_str(), 400, Some("damaged".to_owned()))
        .await
        .unwrap();
    // Only surrounding whitespace differs: the normalized reason must map
    // to the same deterministic idempotency key.
    let second = service
        .refund(payment.id.as_str(), 400, Some(" damaged ".to_owned()))
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].id, first.id);
}

#[tokio::test]
async fn omitted_refund_key_is_derived_and_idempotent() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("derived"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("derived", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(payment.id.as_str(), 300, Some("goodwill".to_owned()), None)
        .await
        .unwrap();
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            300,
            Some(" goodwill ".to_owned()),
            None,
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(provider.refund_count(), 1);
    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
}

#[tokio::test]
async fn duplicate_full_refund_key_returns_existing_after_payment_is_refunded() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe-full"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("dedupe-full", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let first = service
        .refund_with_idempotency(
            payment.id.as_str(),
            1000,
            None,
            Some("return-refund:full".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
    let second = service
        .refund_with_idempotency(
            payment.id.as_str(),
            1000,
            None,
            Some("return-refund:full".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(provider.refund_count(), 1);
}

#[tokio::test]
async fn concurrent_duplicate_refund_keys_return_the_winner() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("dedupe-concurrent"));
    service.register_provider(provider.clone()).unwrap();
    let session = service
        .create_session(input("dedupe-concurrent", 1000))
        .await
        .unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();
    let service = Arc::new(service);

    let first = service.clone();
    let second = service.clone();
    let payment_id = payment.id.to_string();
    let (left, right) = tokio::join!(
        first.refund_with_idempotency(
            &payment_id,
            400,
            None,
            Some("return-refund:race".to_owned())
        ),
        second.refund_with_idempotency(
            &payment_id,
            400,
            None,
            Some("return-refund:race".to_owned())
        )
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(
        service
            .list_refunds(payment_id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn pending_refund_crash_retry_calls_the_provider_and_reconciles() {
    let (service, db) = setup().await;
    let provider = Arc::new(TestProvider::new("crash"));
    service.register_provider(provider.clone()).unwrap();
    let session = service.create_session(input("crash", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    // Simulate a crash after the pending refund row was committed but
    // before the provider call returned.
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_crash', ?, NULL, 400, 'USD', 'pending', NULL, \
                     'return-refund:crash', NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();

    let refund = service
        .refund_with_idempotency(
            payment.id.as_str(),
            400,
            None,
            Some("return-refund:crash".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(refund.id.as_str(), "refund_crash");
    assert_eq!(refund.status, RefundStatus::Succeeded);
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );
}

#[tokio::test]
async fn refund_failed_webhook_reverts_the_payment_status() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              idempotency_key, reason, created_at, updated_at) \
             VALUES ('refund_revert', ?, NULL, 400, 'USD', 'succeeded', \
                     'manual_refund_revert', NULL, NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE payments SET status = 'partially_refunded' WHERE id = ?")
        .bind(payment.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let body = r#"{"id":"evt_refund_failed","type":"refund.failed","data":{"provider_refund_id":"manual_refund_revert","provider_payment_id":"unused"}}"#;
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Failed);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn webhook_with_valid_signature_updates_session_state() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_captured_1","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.event_type, "payment.captured");
    assert_eq!(record.attempts, 1);
    assert!(record.processed_at.is_some());
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn failed_webhooks_are_retried_instead_of_being_dropped() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    sqlx::query(
        "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_retry', 'manual', 'evt_retry', 'payment.captured', '{}',
                     'failed', 1, NULL, 'transient', 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 2);
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn received_webhook_is_applied_by_a_later_delivery() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_received","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    // Simulate a first delivery that inserted the row and then crashed
    // before applying the event.
    sqlx::query(
        "INSERT INTO payment_webhooks
             (id, provider, provider_event_id, event_type, payload, status, attempts,
              processed_at, last_error, created_at)
             VALUES ('wh_received', 'manual', 'evt_received', 'payment.captured', '{}',
                     'received', 0, NULL, NULL, 0)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(record.id.as_str(), "wh_received");
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 1);
    assert_eq!(
        service
            .get_session(session.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn webhook_with_invalid_signature_is_rejected() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let body = br#"{"id":"evt_1","type":"payment.captured","data":{}}"#;

    let error = service
        .process_webhook("manual", headers("wrong-secret", body), body)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Unauthorized));
    let listed = service
        .list_webhooks(None, Pagination::default())
        .await
        .unwrap();
    assert_eq!(listed.count, 0);
}

#[tokio::test]
async fn duplicate_webhook_is_idempotent() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_dup","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    let first = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    let second = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.attempts, 1);
    assert_eq!(second.status, WebhookStatus::Processed);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_webhooks")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn webhook_capture_creates_one_payment_and_is_idempotent_on_replay() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_async","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
    );

    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);

    let payment = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(
        payment.provider_payment_id.as_deref(),
        Some("manual_payment_1")
    );
    assert_eq!(payment.amount, 1000);
    assert_eq!(payment.currency_code, "USD");
    assert_eq!(payment.status, PaymentStatus::Captured);
    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
    assert_eq!(
        updated.data.get("payment_id"),
        Some(&serde_json::json!(payment.id.as_str()))
    );

    // Replaying the exact same provider event must not create another row.
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    // A different event with the same provider identifiers is also
    // idempotent: the unique provider payment id wins.
    let replay = format!(
        r#"{{"id":"evt_async_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_1","amount":1000,"currency":"USD"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", replay.as_bytes()),
            replay.as_bytes(),
        )
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments WHERE provider = 'manual'")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let again = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(again.id, payment.id);
}

#[tokio::test]
async fn webhook_capture_emits_the_captured_event_once() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut session_input = input("manual", 1000);
    let order_id = OrderId::new();
    session_input.order_id = Some(order_id.clone());
    let session = service.create_session(session_input).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_event_once","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    // A redelivery under a different provider event id is still the same
    // capture and must not emit a second lifecycle event.
    let replay = format!(
        r#"{{"id":"evt_event_once_2","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_event"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", replay.as_bytes()),
            replay.as_bytes(),
        )
        .await
        .unwrap();

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM outbox WHERE event_type = 'payment.captured'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
    let payload: String =
        sqlx::query_scalar("SELECT payload FROM outbox WHERE event_type = 'payment.captured'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["order_id"], order_id.as_str());
    assert!(payload["payment_id"].as_str().is_some());
}

#[tokio::test]
async fn webhook_capture_reconciles_a_failed_session() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    service
        .fail_session(session.id.as_str(), "previous ambiguity")
        .await
        .unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_recover","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}","provider_payment_id":"manual_payment_recover","amount":1000}}}}"#
    );

    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    let updated = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(updated.status, PaymentStatus::Captured);
    let payment = service
        .payment_for_session_id(session.id.as_str())
        .await
        .unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(
        payment.provider_payment_id.as_deref(),
        Some("manual_payment_recover")
    );
}

#[tokio::test]
async fn webhook_apply_failure_is_retryable_and_reapplied_on_redelivery() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_apply_retry","type":"payment.captured","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    sqlx::query(
        "CREATE TRIGGER reject_capture BEFORE UPDATE ON payment_sessions \
             WHEN NEW.status = 'captured' BEGIN SELECT RAISE(ABORT, 'transient'); END",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let error = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    let status: String = sqlx::query_scalar(
        "SELECT status FROM payment_webhooks WHERE provider_event_id = 'evt_apply_retry'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(status, "failed");

    sqlx::query("DROP TRIGGER reject_capture")
        .execute(db.pool())
        .await
        .unwrap();
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.attempts, 2);
    assert_eq!(
        service
            .get_session(session.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Captured
    );
}

#[tokio::test]
async fn unknown_webhook_event_is_stored_without_state_changes() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let body = br#"{"id":"evt_unknown","type":"customer.created","data":{"customer_id":"cust_1"}}"#;
    let record = service
        .process_webhook("manual", headers("secret", body), body)
        .await
        .unwrap();

    assert_eq!(record.status, WebhookStatus::Processed);
    assert_eq!(record.event_type, "customer.created");
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
}

#[tokio::test]
async fn payment_failed_webhook_marks_session_failed() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let provider_session_id = session.provider_session_id.clone().unwrap();
    let body = format!(
        r#"{{"id":"evt_failed","type":"payment.failed","data":{{"provider_session_id":"{provider_session_id}"}}}}"#
    );
    service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();

    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Failed);
}

#[tokio::test]
async fn refund_succeeded_webhook_updates_refund_and_payment() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    // Persist a pending refund directly so the webhook has a row to update.
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              reason, created_at, updated_at) \
             VALUES ('refund_pending', ?, NULL, 400, 'USD', 'pending', 'manual_refund_1', \
              NULL, ?, ?)",
    )
    .bind(payment.id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();

    let body = r#"{"id":"evt_refund","type":"refund.succeeded","data":{"provider_refund_id":"manual_refund_1"}}"#;
    let record = service
        .process_webhook(
            "manual",
            headers("secret", body.as_bytes()),
            body.as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(record.status, WebhookStatus::Processed);

    let refunds = service.list_refunds(payment.id.as_str()).await.unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Succeeded);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );
}

#[tokio::test]
async fn cancel_cannot_overwrite_a_session_captured_after_the_read() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();

    // Simulate cancel observing `pending`, then a capture completing
    // before the stale cancel write is issued.
    let stale = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stale.status, PaymentStatus::Pending);
    let payment = service.capture(session.id.as_str()).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);

    let error = service
        .transition_session(&stale, PaymentStatus::Canceled, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn cancel_conflicts_while_a_capture_claim_is_live() {
    let (service, db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let session = service.create_session(input("manual", 1000)).await.unwrap();
    let now = now_timestamp();
    // Simulate capture() having claimed the session but not yet completed
    // its provider call. The status is still `pending`.
    sqlx::query(
        "UPDATE payment_sessions
             SET data = json_set(data, '$.capture_claim', 'capture:live',
                                 '$.capture_claim_at', ?),
                 updated_at = ?
             WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(session.id.as_str())
    .execute(db.pool())
    .await
    .unwrap();

    let error = service
        .cancel_session(session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    let stored = service.get_session(session.id.as_str()).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Pending);
    assert!(stored.data.get("capture_claim").is_some());
}

#[tokio::test]
async fn fail_and_cancel_respect_the_state_machine() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();

    let canceled_session = service.create_session(input("manual", 100)).await.unwrap();
    let canceled = service
        .cancel_session(canceled_session.id.as_str())
        .await
        .unwrap();
    assert_eq!(canceled.status, PaymentStatus::Canceled);
    let capture_error = service
        .capture(canceled_session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(capture_error, AppError::Conflict(_)));

    let failed_session = service.create_session(input("manual", 100)).await.unwrap();
    let failed = service
        .fail_session(failed_session.id.as_str(), "declined")
        .await
        .unwrap();
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(
        failed.data.get("last_error"),
        Some(&serde_json::json!("declined"))
    );

    let captured_session = service.create_session(input("manual", 100)).await.unwrap();
    service.capture(captured_session.id.as_str()).await.unwrap();
    let cancel_error = service
        .cancel_session(captured_session.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(cancel_error, AppError::Conflict(_)));
}

#[tokio::test]
async fn lists_payments_with_order_and_status_filters() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut first_input = input("manual", 500);
    first_input.order_id = Some(OrderId::new());
    let first = service.create_session(first_input).await.unwrap();
    service.capture(first.id.as_str()).await.unwrap();

    let mut second_input = input("manual", 700);
    second_input.order_id = Some(OrderId::new());
    let second = service.create_session(second_input).await.unwrap();
    service.capture(second.id.as_str()).await.unwrap();

    let by_order = service
        .list_payments(ListPaymentsFilter::new().with_order(first.order_id.clone().unwrap()))
        .await
        .unwrap();
    assert_eq!(by_order.count, 1);
    assert_eq!(by_order.data[0].amount, 500);

    let captured = service
        .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Captured))
        .await
        .unwrap();
    assert_eq!(captured.count, 2);

    let page = service
        .list_payments(ListPaymentsFilter {
            pagination: Pagination::new(Some(1), Some(1)).unwrap(),
            ..ListPaymentsFilter::new()
        })
        .await
        .unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.count, 2);
    assert_eq!(page.pagination.limit, 1);

    let empty: PageResult<crate::domain::Payment> = service
        .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Refunded))
        .await
        .unwrap();
    assert!(empty.data.is_empty());
}

#[tokio::test]
async fn missing_resources_return_not_found() {
    let (service, _db) = setup().await;
    assert!(matches!(
        service.get_session("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.get_payment("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.get_webhook("missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn webhook_unknown_provider_is_rejected() {
    let (service, _db) = setup().await;
    let error = service
        .process_webhook("missing", BTreeMap::new(), b"{}")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn payment_metadata_is_copied_from_the_session() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut metadata = Metadata::empty();
    metadata.insert("source", serde_json::json!("storefront"));
    let mut session_input = input("manual", 990);
    session_input.metadata = metadata;
    let session = service.create_session(session_input).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();

    assert_eq!(
        payment.metadata.get("source"),
        Some(&serde_json::json!("storefront"))
    );
}
