//! Partial capture behaviour of [`PaymentService`].
//!
//! Capture used to always take the session amount in one shot, which forces a
//! full refund and re-charge whenever only part of an order can be fulfilled.
//! These tests pin the partial-capture contract:
//!
//! * a capture may take less than the session amount and the payment records
//!   what was actually captured,
//! * repeated captures accumulate up to the session amount and never past it,
//! * the cumulative cap is enforced inside the same transaction that claims the
//!   capture, so concurrent callers cannot over-capture,
//! * a caller idempotency key replays the original payment instead of charging
//!   twice,
//! * refunds stay capped by the amount actually captured.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession, Payment,
    PaymentError, PaymentProvider, PaymentService, PaymentStatus, ProviderSession, RefundRequest,
    RefundResult, RefundStatus, VerifiedWebhook,
};
use tokio::sync::Barrier;

/// Provider that records capture requests and can hold a capture open so the
/// claim window is wide enough to overlap with concurrent callers.
///
/// Like Stripe's Payment Intent, one provider payment id identifies the whole
/// session, so partial captures accumulate on it.
struct TestProvider {
    id: String,
    capture_delay: Duration,
    reject_captures: AtomicBool,
    ambiguous_failures: AtomicUsize,
    sessions: AtomicUsize,
    capture_calls: AtomicUsize,
    refund_calls: AtomicUsize,
    captured_amounts: std::sync::Mutex<Vec<i64>>,
    capture_keys: std::sync::Mutex<Vec<Option<String>>>,
}

impl TestProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            capture_delay: Duration::ZERO,
            reject_captures: AtomicBool::new(false),
            ambiguous_failures: AtomicUsize::new(0),
            sessions: AtomicUsize::new(0),
            capture_calls: AtomicUsize::new(0),
            refund_calls: AtomicUsize::new(0),
            captured_amounts: std::sync::Mutex::new(Vec::new()),
            capture_keys: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_capture_delay(id: &str, capture_delay: Duration) -> Self {
        Self {
            capture_delay,
            ..Self::new(id)
        }
    }

    fn reject_captures(&self, reject: bool) {
        self.reject_captures.store(reject, Ordering::SeqCst);
    }

    /// Makes the next `count` captures fail with an ambiguous provider error,
    /// as a lost response would.
    fn fail_captures_ambiguously(&self, count: usize) {
        self.ambiguous_failures.store(count, Ordering::SeqCst);
    }

    fn capture_count(&self) -> usize {
        self.capture_calls.load(Ordering::SeqCst)
    }

    fn refund_count(&self) -> usize {
        self.refund_calls.load(Ordering::SeqCst)
    }

    fn captured_amounts(&self) -> Vec<i64> {
        self.captured_amounts.lock().unwrap().clone()
    }

    fn capture_keys(&self) -> Vec<Option<String>> {
        self.capture_keys.lock().unwrap().clone()
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
        let index = self.sessions.fetch_add(1, Ordering::SeqCst);
        Ok(ProviderSession {
            provider_session_id: format!("{}_session_{index}", self.id),
            status: PaymentStatus::Pending,
            data: serde_json::json!({}),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        self.capture_calls.fetch_add(1, Ordering::SeqCst);
        self.capture_keys
            .lock()
            .unwrap()
            .push(request.idempotency_key.clone());
        if self.reject_captures.load(Ordering::SeqCst) {
            return Err(PaymentError::InvalidRequest(
                "capture request is invalid".to_owned(),
            ));
        }
        if self.ambiguous_failures.load(Ordering::SeqCst) > 0 {
            self.ambiguous_failures.fetch_sub(1, Ordering::SeqCst);
            return Err(PaymentError::Provider("capture transport error".to_owned()));
        }
        self.captured_amounts.lock().unwrap().push(request.amount);
        if !self.capture_delay.is_zero() {
            tokio::time::sleep(self.capture_delay).await;
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
        Ok(RefundResult {
            provider_refund_id: format!("{}_refund_{}", self.id, self.refund_count()),
            amount: request.amount,
            currency_code: request.currency_code,
            status: RefundStatus::Succeeded,
        })
    }

    fn verify_webhook(
        &self,
        _headers: &std::collections::BTreeMap<String, String>,
        _body: &[u8],
    ) -> Result<VerifiedWebhook, PaymentError> {
        Err(PaymentError::InvalidSignature)
    }
}

async fn setup(provider: Arc<TestProvider>) -> (PaymentService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PaymentService::new(db.clone());
    service.register_provider(provider).unwrap();
    (service, db)
}

async fn setup_multi_connection(
    provider: Arc<TestProvider>,
    max_connections: u32,
) -> (tempfile::TempDir, StoreDatabase, Arc<PaymentService>) {
    let directory = tempfile::tempdir().expect("temp dir");
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        max_connections,
    )
    .await
    .expect("multi-connection store database");
    let service = PaymentService::new(db.clone());
    service
        .register_provider(provider)
        .expect("register provider");
    (directory, db, Arc::new(service))
}

async fn create_session(service: &PaymentService, amount: i64) -> String {
    service
        .create_session(CreatePaymentSessionInput::new("partial", amount, "usd"))
        .await
        .expect("create session")
        .id
        .to_string()
}

#[tokio::test]
async fn partial_capture_records_the_partial_amount_and_status() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let payment = service.capture_amount(&session_id, 600).await.unwrap();

    assert_eq!(payment.amount, 600);
    assert_eq!(payment.status, PaymentStatus::PartiallyCaptured);
    assert!(payment.captured_at.is_some());
    assert_eq!(provider.captured_amounts(), vec![600]);

    let stored_payment = service.get_payment(payment.id.as_str()).await.unwrap();
    assert_eq!(stored_payment.amount, 600);
    assert_eq!(stored_payment.status, PaymentStatus::PartiallyCaptured);

    let session = service.get_session(&session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::PartiallyCaptured);

    let attempts = service.list_attempts(&session_id).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].amount, 600);
    assert_eq!(attempts[1].status, PaymentStatus::Captured);
}

#[tokio::test]
async fn captures_accumulate_across_calls_until_the_full_amount() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let first = service.capture_amount(&session_id, 400).await.unwrap();
    assert_eq!(first.amount, 400);
    assert_eq!(first.status, PaymentStatus::PartiallyCaptured);

    let second = service.capture_amount(&session_id, 600).await.unwrap();
    assert_eq!(second.id, first.id, "one payment accumulates the captures");
    assert_eq!(second.amount, 1000);
    assert_eq!(second.status, PaymentStatus::Captured);
    assert_eq!(provider.captured_amounts(), vec![400, 600]);

    let stored = service.get_payment(first.id.as_str()).await.unwrap();
    assert_eq!(stored.amount, 1000);
    assert_eq!(stored.status, PaymentStatus::Captured);
    assert_eq!(
        service.get_session(&session_id).await.unwrap().status,
        PaymentStatus::Captured
    );
    assert_eq!(service.list_attempts(&session_id).await.unwrap().len(), 3);
}

#[tokio::test]
async fn over_capture_is_rejected_and_leaves_state_unchanged() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let over = service.capture_amount(&session_id, 1001).await.unwrap_err();
    assert!(matches!(over, AppError::Validation(_)));
    assert_eq!(provider.capture_count(), 0);
    let session = service.get_session(&session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::Pending);

    let payment = service.capture_amount(&session_id, 600).await.unwrap();
    assert_eq!(payment.amount, 600);

    // The cumulative cap, not just the session amount, is enforced.
    let over = service.capture_amount(&session_id, 401).await.unwrap_err();
    assert!(matches!(over, AppError::Validation(_)));
    assert_eq!(provider.capture_count(), 1);
    let stored = service.get_payment(payment.id.as_str()).await.unwrap();
    assert_eq!(stored.amount, 600);
    assert_eq!(stored.status, PaymentStatus::PartiallyCaptured);
    assert_eq!(
        service.get_session(&session_id).await.unwrap().status,
        PaymentStatus::PartiallyCaptured
    );

    // The exact remainder is still allowed.
    let completed = service.capture_amount(&session_id, 400).await.unwrap();
    assert_eq!(completed.amount, 1000);
    assert_eq!(completed.status, PaymentStatus::Captured);
}

#[tokio::test]
async fn zero_and_negative_amounts_are_rejected_without_calling_the_provider() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    for amount in [0, -1] {
        let error = service
            .capture_amount(&session_id, amount)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)), "{amount}");
    }

    assert_eq!(provider.capture_count(), 0);
    let session = service.get_session(&session_id).await.unwrap();
    assert_eq!(session.status, PaymentStatus::Pending);
    assert!(session.data.get("payment_id").is_none());
}

#[tokio::test]
async fn duplicated_idempotency_key_returns_the_original_payment() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let first = service
        .capture_amount_with_idempotency(&session_id, 300, Some("shipment-1".to_owned()))
        .await
        .unwrap();
    let replay = service
        .capture_amount_with_idempotency(&session_id, 300, Some("shipment-1".to_owned()))
        .await
        .unwrap();

    assert_eq!(replay.id, first.id);
    assert_eq!(replay.amount, 300);
    assert_eq!(provider.capture_count(), 1, "the key must not charge twice");
    assert_eq!(
        service.get_payment(first.id.as_str()).await.unwrap().amount,
        300
    );

    // Reusing the key for a different amount would silently drop that capture.
    let conflicting = service
        .capture_amount_with_idempotency(&session_id, 200, Some("shipment-1".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(conflicting, AppError::Conflict(_)));
    assert_eq!(provider.capture_count(), 1);

    // A fresh key captures again and accumulates.
    let second = service
        .capture_amount_with_idempotency(&session_id, 300, Some("shipment-2".to_owned()))
        .await
        .unwrap();
    assert_eq!(second.id, first.id);
    assert_eq!(second.amount, 600);
    assert_eq!(provider.capture_count(), 2);
}

#[tokio::test]
async fn partial_capture_then_partial_refund_is_capped_by_the_captured_amount() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let payment = service.capture_amount(&session_id, 600).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::PartiallyCaptured);

    let refunded = service
        .refund(payment.id.as_str(), 400, Some("missing item".to_owned()))
        .await
        .unwrap();
    assert_eq!(refunded.status, RefundStatus::Succeeded);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::PartiallyRefunded
    );

    // 400 of 600 is refunded; another 300 would exceed what was captured.
    let over = service
        .refund(payment.id.as_str(), 300, Some("more".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)));
    assert_eq!(provider.refund_count(), 1);

    let remainder = service
        .refund(payment.id.as_str(), 200, Some("rest".to_owned()))
        .await
        .unwrap();
    assert_eq!(remainder.status, RefundStatus::Succeeded);
    assert_eq!(
        service
            .get_payment(payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
    assert_eq!(
        service
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .iter()
            .map(|refund| refund.amount)
            .sum::<i64>(),
        600
    );
}

#[tokio::test]
async fn legacy_capture_completes_a_partially_captured_session() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let first = service.capture_amount(&session_id, 400).await.unwrap();
    assert_eq!(first.amount, 400);

    // The unchanged entry point still captures the full remaining amount.
    let completed = service.capture(&session_id).await.unwrap();
    assert_eq!(completed.id, first.id);
    assert_eq!(completed.amount, 1000);
    assert_eq!(completed.status, PaymentStatus::Captured);
    assert_eq!(provider.captured_amounts(), vec![400, 600]);

    // Replaying the legacy capture returns the completed payment.
    let replay = service.capture(&session_id).await.unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(provider.capture_count(), 2);
}

#[tokio::test]
async fn ambiguous_failure_keeps_its_reservation_and_reuses_the_provider_key() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    provider.fail_captures_ambiguously(1);
    let error = service.capture_amount(&session_id, 400).await.unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));

    // The capture may have succeeded at the provider, so its amount stays
    // reserved: the rest of the session is only 600, not 1000.
    let over = service
        .capture_amount_with_idempotency(&session_id, 700, Some("other".to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Validation(_)));
    assert_eq!(
        provider.capture_count(),
        1,
        "the rejected call never charged"
    );

    // Retrying the same logical capture reuses the provider idempotency key, so
    // the provider de-duplicates it instead of charging twice.
    let payment = service.capture_amount(&session_id, 400).await.unwrap();
    assert_eq!(payment.amount, 400);
    let keys = provider.capture_keys();
    assert_eq!(keys.len(), 2);
    assert_eq!(
        keys[0], keys[1],
        "a retry must reuse the provider idempotency key"
    );
    assert!(keys[0].is_some());
    assert_eq!(provider.captured_amounts(), vec![400]);
    assert_eq!(
        service.get_session(&session_id).await.unwrap().status,
        PaymentStatus::PartiallyCaptured
    );
}

#[tokio::test]
async fn partially_captured_sessions_cannot_be_canceled_or_failed() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;
    service.capture_amount(&session_id, 600).await.unwrap();

    // The session holds captured money: whether it can still reach the
    // remaining amount is an open question, but it cannot be silently dropped.
    for error in [
        service.cancel_session(&session_id).await.unwrap_err(),
        service
            .fail_session(&session_id, "declined")
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    }
    assert_eq!(
        service.get_session(&session_id).await.unwrap().status,
        PaymentStatus::PartiallyCaptured
    );
}

#[tokio::test]
async fn definitive_rejection_releases_the_reservation_and_keeps_the_capture() {
    let provider = Arc::new(TestProvider::new("partial"));
    let (service, _db) = setup(provider.clone()).await;
    let session_id = create_session(&service, 1000).await;

    let first = service.capture_amount(&session_id, 600).await.unwrap();
    assert_eq!(first.amount, 600);

    provider.reject_captures(true);
    let rejected = service.capture_amount(&session_id, 400).await.unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)));

    // A rejected capture of an already captured session must not throw the
    // captured money away.
    let stored = service.get_session(&session_id).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::PartiallyCaptured);
    assert_eq!(
        service.get_payment(first.id.as_str()).await.unwrap().amount,
        600
    );
    assert!(
        service
            .list_attempts(&session_id)
            .await
            .unwrap()
            .iter()
            .any(|attempt| attempt.status == PaymentStatus::Failed)
    );

    // The rejection released the reservation, so the remainder still fits.
    provider.reject_captures(false);
    let completed = service.capture_amount(&session_id, 400).await.unwrap();
    assert_eq!(completed.amount, 1000);
    assert_eq!(completed.status, PaymentStatus::Captured);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_partial_captures_never_exceed_the_session_amount() {
    let provider = Arc::new(TestProvider::with_capture_delay(
        "partial",
        Duration::from_millis(40),
    ));
    let (_directory, db, service) = setup_multi_connection(provider.clone(), 8).await;
    let session = service
        .create_session(CreatePaymentSessionInput::new("partial", 1000, "usd"))
        .await
        .expect("create session");
    let session_id = session.id.to_string();

    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = Vec::new();
    for index in 0..4 {
        let service = service.clone();
        let barrier = barrier.clone();
        let session_id = session_id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .capture_amount_with_idempotency(&session_id, 400, Some(format!("racer-{index}")))
                .await
        }));
    }

    let mut winners: Vec<Payment> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    for task in tasks {
        match task.await.expect("capture task panicked") {
            Ok(payment) => winners.push(payment),
            Err(error) => failures.push(error),
        }
    }

    assert!(
        !winners.is_empty(),
        "at least one partial capture must succeed: {failures:?}"
    );
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, AppError::Conflict(_) | AppError::Validation(_))),
        "a losing capture must fail cleanly, got: {failures:?}"
    );
    let payment_id = winners[0].id.clone();
    assert!(
        winners.iter().all(|payment| payment.id == payment_id),
        "partial captures accumulate on one payment"
    );
    let captured = 400 * winners.len() as i64;
    assert!(
        captured <= 1000,
        "the sum of concurrent partial captures must never exceed the session amount"
    );
    assert_eq!(
        provider.capture_count(),
        winners.len(),
        "the provider must be called once per successful capture"
    );

    let payment = service.get_payment(payment_id.as_str()).await.unwrap();
    assert_eq!(payment.amount, captured);
    assert_eq!(provider.captured_amounts().iter().sum::<i64>(), captured);
    let stored = service.get_session(&session_id).await.unwrap();
    assert_eq!(
        stored.status,
        if captured == 1000 {
            PaymentStatus::Captured
        } else {
            PaymentStatus::PartiallyCaptured
        }
    );

    let attempts = service.list_attempts(&session_id).await.unwrap();
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.status == PaymentStatus::Captured)
            .count(),
        winners.len()
    );
    let payment_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        payment_rows, 1,
        "one payment per session, whatever the race"
    );
}
