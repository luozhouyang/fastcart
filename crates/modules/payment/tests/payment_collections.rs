//! Payment collections: the money a checkout owes, paid by one or more
//! sessions.
//!
//! A collection is the anchor for combination payments. A checkout can be paid
//! with several sessions (for example two cards) and only counts as paid when
//! the captured amounts add up to the collection amount:
//!
//! * creating a collection for a checkout is idempotent,
//! * a session only belongs to one collection and only in the collection's
//!   currency,
//! * capturing a collection captures every session it holds and refuses to
//!   report success unless the captured total matches the collection amount,
//! * a collection that is not covered by its sessions is rejected before any
//!   provider is called, so a failed combination payment never charges a part
//!   of the money it cannot settle,
//! * concurrent collection captures never capture a session twice.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession, Payment,
    PaymentCollectionStatus, PaymentError, PaymentProvider, PaymentService, PaymentStatus,
    ProviderSession, RefundRequest, RefundResult, RefundStatus, VerifiedWebhook,
};
use tokio::sync::Barrier;

/// Minimal provider that counts its calls and records the captured amounts.
struct CountingProvider {
    id: String,
    capture_delay: Duration,
    sessions: AtomicUsize,
    capture_calls: AtomicUsize,
    captured_amounts: std::sync::Mutex<Vec<i64>>,
}

impl CountingProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            capture_delay: Duration::ZERO,
            sessions: AtomicUsize::new(0),
            capture_calls: AtomicUsize::new(0),
            captured_amounts: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_capture_delay(id: &str, capture_delay: Duration) -> Self {
        Self {
            capture_delay,
            ..Self::new(id)
        }
    }

    fn capture_count(&self) -> usize {
        self.capture_calls.load(Ordering::SeqCst)
    }

    fn captured_amounts(&self) -> Vec<i64> {
        self.captured_amounts.lock().unwrap().clone()
    }
}

#[async_trait]
impl PaymentProvider for CountingProvider {
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
        self.captured_amounts.lock().unwrap().push(request.amount);
        if !self.capture_delay.is_zero() {
            tokio::time::sleep(self.capture_delay).await;
        }
        Ok(CaptureResult {
            // Like Stripe's Payment Intent, one provider payment object belongs
            // to one provider session, so two sessions of the same provider are
            // two provider payments.
            provider_payment_id: format!("{}_payment_{}", self.id, request.provider_session_id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: PaymentStatus::Captured,
        })
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        Ok(RefundResult {
            provider_refund_id: format!("{}_refund", self.id),
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

async fn setup() -> (PaymentService, StoreDatabase, Arc<CountingProvider>) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PaymentService::new(db.clone());
    let provider = Arc::new(CountingProvider::new("collections"));
    service.register_provider(provider.clone()).unwrap();
    (service, db, provider)
}

async fn setup_multi_connection(
    max_connections: u32,
    capture_delay: Duration,
) -> (
    tempfile::TempDir,
    StoreDatabase,
    Arc<PaymentService>,
    Arc<CountingProvider>,
) {
    let directory = tempfile::tempdir().expect("temp dir");
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        max_connections,
    )
    .await
    .expect("multi-connection store database");
    let service = PaymentService::new(db.clone());
    let provider = Arc::new(CountingProvider::with_capture_delay(
        "collections",
        capture_delay,
    ));
    service
        .register_provider(provider.clone())
        .expect("register provider");
    (directory, db, Arc::new(service), provider)
}

async fn create_session(service: &PaymentService, amount: i64) -> String {
    service
        .create_session(CreatePaymentSessionInput::new("collections", amount, "usd"))
        .await
        .expect("create session")
        .id
        .to_string()
}

async fn create_collection(service: &PaymentService, checkout_id: &str, amount: i64) -> String {
    service
        .create_collection(checkout_id, amount, "usd")
        .await
        .expect("create collection")
        .id
        .to_string()
}

async fn count_rows(db: &StoreDatabase, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db.pool())
        .await
        .expect("count rows")
}

async fn captured_total(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(SUM(amount), 0) FROM payments")
        .fetch_one(db.pool())
        .await
        .expect("sum payments")
}

fn payment_ids(payments: &[Payment]) -> Vec<String> {
    payments
        .iter()
        .map(|payment| payment.id.to_string())
        .collect()
}

#[tokio::test]
async fn creating_a_collection_for_a_checkout_is_idempotent() {
    let (service, db, _provider) = setup().await;

    let first = service
        .create_collection("checkout-1", 1_000, "usd")
        .await
        .unwrap();
    assert_eq!(first.status, PaymentCollectionStatus::Open);
    assert_eq!(first.amount, 1_000);
    assert_eq!(first.currency_code, "USD");

    let replay = service
        .create_collection("checkout-1", 1_000, "USD")
        .await
        .unwrap();
    assert_eq!(replay.id, first.id, "a checkout has one collection");
    assert_eq!(count_rows(&db, "payment_collections").await, 1);

    // The same checkout cannot be re-priced under a caller's feet.
    for error in [
        service
            .create_collection("checkout-1", 900, "usd")
            .await
            .unwrap_err(),
        service
            .create_collection("checkout-1", 1_000, "eur")
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    }
    assert_eq!(count_rows(&db, "payment_collections").await, 1);

    // A different checkout gets its own collection.
    let other = service
        .create_collection("checkout-2", 1_000, "usd")
        .await
        .unwrap();
    assert_ne!(other.id, first.id);
    assert_eq!(count_rows(&db, "payment_collections").await, 2);
}

#[tokio::test]
async fn creating_a_collection_validates_its_amount_and_currency() {
    let (service, db, _provider) = setup().await;

    assert!(matches!(
        service
            .create_collection("checkout-1", -1, "usd")
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
    assert!(matches!(
        service
            .create_collection("checkout-1", 1_000, "usdollars")
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
    assert_eq!(count_rows(&db, "payment_collections").await, 0);
}

#[tokio::test]
async fn a_session_is_attached_in_the_collection_currency_only() {
    let (service, db, _provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;

    let matching = create_session(&service, 400).await;
    service
        .attach_session_to_collection(&collection, &matching)
        .await
        .unwrap();
    assert_eq!(count_rows(&db, "payment_collection_sessions").await, 1);

    let foreign = service
        .create_session(CreatePaymentSessionInput::new("collections", 400, "eur"))
        .await
        .unwrap();
    let error = service
        .attach_session_to_collection(&collection, foreign.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    assert_eq!(
        count_rows(&db, "payment_collection_sessions").await,
        1,
        "a rejected session must not be linked"
    );
}

#[tokio::test]
async fn a_session_cannot_belong_to_two_collections() {
    let (service, db, _provider) = setup().await;
    let first = create_collection(&service, "checkout-1", 1_000).await;
    let second = create_collection(&service, "checkout-2", 1_000).await;
    let session = create_session(&service, 400).await;

    service
        .attach_session_to_collection(&first, &session)
        .await
        .unwrap();
    // Attaching the same session to the same collection is idempotent.
    service
        .attach_session_to_collection(&first, &session)
        .await
        .unwrap();
    assert_eq!(count_rows(&db, "payment_collection_sessions").await, 1);

    let error = service
        .attach_session_to_collection(&second, &session)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(count_rows(&db, "payment_collection_sessions").await, 1);
}

#[tokio::test]
async fn attaching_unknown_resources_is_rejected() {
    let (service, _db, _provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;

    assert!(matches!(
        service
            .attach_session_to_collection("pcoll_missing", "psess_missing")
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service
            .attach_session_to_collection(&collection, "psess_missing")
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service.get_collection("pcoll_missing").await.unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service
            .capture_collection("pcoll_missing")
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service
            .collection_status("pcoll_missing")
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
}

#[tokio::test]
async fn a_combination_payment_captures_every_session() {
    let (service, db, provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;
    let first = create_session(&service, 400).await;
    let second = create_session(&service, 600).await;
    service
        .attach_session_to_collection(&collection, &first)
        .await
        .unwrap();
    service
        .attach_session_to_collection(&collection, &second)
        .await
        .unwrap();
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Open
    );

    let payments = service.capture_collection(&collection).await.unwrap();

    assert_eq!(payments.len(), 2);
    assert_eq!(
        payments.iter().map(|payment| payment.amount).sum::<i64>(),
        1_000,
        "the captures must add up to the collection amount"
    );
    assert!(
        payments
            .iter()
            .all(|payment| payment.status == PaymentStatus::Captured),
        "every captured payment is complete: {payments:?}"
    );
    assert_eq!(provider.captured_amounts(), vec![400, 600]);
    assert_eq!(count_rows(&db, "payments").await, 2);
    for session in [&first, &second] {
        assert_eq!(
            service.get_session(session).await.unwrap().status,
            PaymentStatus::Captured
        );
    }
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Captured
    );

    // Replaying the capture returns the recorded payments without charging
    // again.
    let replay = service.capture_collection(&collection).await.unwrap();
    assert_eq!(payment_ids(&replay), payment_ids(&payments));
    assert_eq!(provider.capture_count(), 2);
    assert_eq!(captured_total(&db).await, 1_000);
}

#[tokio::test]
async fn an_under_covered_collection_is_rejected_before_any_capture() {
    let (service, db, provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;
    let session = create_session(&service, 600).await;
    service
        .attach_session_to_collection(&collection, &session)
        .await
        .unwrap();

    let error = service.capture_collection(&collection).await.unwrap_err();

    assert!(
        matches!(
            &error,
            AppError::Conflict(message)
                if message.contains("400") && message.contains("under-captured")
        ),
        "the shortfall must be reported: {error:?}"
    );
    assert_eq!(
        provider.capture_count(),
        0,
        "a collection that cannot settle anything must not charge anybody"
    );
    assert_eq!(count_rows(&db, "payments").await, 0);
    assert_eq!(
        service.get_session(&session).await.unwrap().status,
        PaymentStatus::Pending
    );
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Open
    );
}

#[tokio::test]
async fn an_over_covered_collection_is_rejected_before_any_capture() {
    let (service, db, provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;
    for amount in [600, 600] {
        let session = create_session(&service, amount).await;
        service
            .attach_session_to_collection(&collection, &session)
            .await
            .unwrap();
    }

    let error = service.capture_collection(&collection).await.unwrap_err();

    assert!(
        matches!(
            &error,
            AppError::Conflict(message)
                if message.contains("200") && message.contains("over-captured")
        ),
        "the excess must be reported: {error:?}"
    );
    assert_eq!(
        provider.capture_count(),
        0,
        "a collection must never capture more than its amount"
    );
    assert_eq!(count_rows(&db, "payments").await, 0);
}

#[tokio::test]
async fn an_empty_collection_cannot_be_captured() {
    let (service, db, provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;

    assert!(matches!(
        service.capture_collection(&collection).await.unwrap_err(),
        AppError::Conflict(_)
    ));
    assert_eq!(provider.capture_count(), 0);
    assert_eq!(count_rows(&db, "payments").await, 0);
}

#[tokio::test]
async fn a_single_session_collection_matches_the_direct_capture_path() {
    let (service, _db, provider) = setup().await;

    // The original path: capture the session directly.
    let legacy_session = create_session(&service, 1_000).await;
    let legacy = service.capture(&legacy_session).await.unwrap();

    // The collection path over the same shape of session.
    let collection = create_collection(&service, "checkout-1", 1_000).await;
    let session = create_session(&service, 1_000).await;
    service
        .attach_session_to_collection(&collection, &session)
        .await
        .unwrap();
    let payments = service.capture_collection(&collection).await.unwrap();

    assert_eq!(payments.len(), 1);
    let payment = &payments[0];
    assert_eq!(payment.amount, legacy.amount);
    assert_eq!(payment.status, legacy.status);
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert!(payment.captured_at.is_some());
    assert_eq!(
        service.payment_for_session_id(&session).await.unwrap().id,
        payment.id,
        "the collection payment is the session payment"
    );
    assert_eq!(
        service.get_session(&session).await.unwrap().status,
        service.get_session(&legacy_session).await.unwrap().status
    );
    assert_eq!(
        service.list_attempts(&session).await.unwrap().len(),
        service.list_attempts(&legacy_session).await.unwrap().len()
    );
    assert_eq!(
        provider.capture_count(),
        2,
        "one provider capture per session"
    );
}

#[tokio::test]
async fn collection_status_tracks_the_captured_amount() {
    let (service, _db, _provider) = setup().await;
    let collection = create_collection(&service, "checkout-1", 1_000).await;
    let first = create_session(&service, 400).await;
    let second = create_session(&service, 600).await;
    service
        .attach_session_to_collection(&collection, &first)
        .await
        .unwrap();
    service
        .attach_session_to_collection(&collection, &second)
        .await
        .unwrap();

    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Open
    );

    // One session is paid outside the collection (for example by an operator
    // completing the remainder of a split payment by hand).
    service.capture_amount(&first, 400).await.unwrap();
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::PartiallyCaptured
    );

    let payments = service.capture_collection(&collection).await.unwrap();
    assert_eq!(
        payments.iter().map(|payment| payment.amount).sum::<i64>(),
        1_000
    );
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Captured
    );
    assert_eq!(
        service.get_collection(&collection).await.unwrap().status,
        PaymentCollectionStatus::Captured,
        "the stored status must not keep claiming the collection is open"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_collection_captures_never_double_charge() {
    let (_directory, db, service, provider) =
        setup_multi_connection(8, Duration::from_millis(40)).await;
    let collection = create_collection(&service, "checkout-race", 1_000).await;
    let first = create_session(&service, 400).await;
    let second = create_session(&service, 600).await;
    service
        .attach_session_to_collection(&collection, &first)
        .await
        .unwrap();
    service
        .attach_session_to_collection(&collection, &second)
        .await
        .unwrap();

    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let service = service.clone();
        let barrier = barrier.clone();
        let collection = collection.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service.capture_collection(&collection).await
        }));
    }

    let mut winners: Vec<Vec<Payment>> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    for task in tasks {
        match task.await.expect("capture task panicked") {
            Ok(payments) => winners.push(payments),
            Err(error) => failures.push(error),
        }
    }

    assert!(
        !winners.is_empty(),
        "at least one collection capture must succeed: {failures:?}"
    );
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, AppError::Conflict(_))),
        "a losing collection capture must fail cleanly, got: {failures:?}"
    );
    let expected = payment_ids(&winners[0]);
    assert_eq!(expected.len(), 2);
    assert!(
        winners
            .iter()
            .all(|payments| payment_ids(payments) == expected),
        "every successful caller must observe the same payments"
    );

    assert_eq!(
        provider.capture_count(),
        2,
        "each session is captured exactly once, whatever the race"
    );
    assert_eq!(count_rows(&db, "payments").await, 2);
    assert_eq!(captured_total(&db).await, 1_000);
    for session in [&first, &second] {
        let captured = service
            .list_attempts(session)
            .await
            .unwrap()
            .into_iter()
            .filter(|attempt| attempt.status == PaymentStatus::Captured)
            .count();
        assert_eq!(captured, 1, "one capture per session");
    }
    assert_eq!(
        service.collection_status(&collection).await.unwrap(),
        PaymentCollectionStatus::Captured
    );
}
