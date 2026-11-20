//! True multi-connection concurrency tests for payment capture and refunds.
//!
//! The unit tests inside `src/service.rs` (`concurrent_captures_never_call_the_provider_twice`,
//! `concurrent_duplicate_refund_keys_return_the_winner`) use `tokio::join!` on
//! a service backed by a single-connection in-memory pool. The two futures
//! therefore run one after the other: the second caller always observes the
//! already-committed result, so the tests never make the capture claim or the
//! refund claim contend. These tests race real tasks against a file-backed
//! multi-connection pool and assert the invariants that the claim logic exists
//! to protect:
//!
//! * one payment per payment session, whatever the race,
//! * the provider is called at most once per capture,
//! * every caller that succeeds returns that same payment,
//! * a duplicated refund key produces exactly one refund row and one refunded
//!   amount.
//!
//! The last test is a deliberate counterexample: it asserts that a naive
//! check-then-write capture *does* record two payments on this harness.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession, Payment,
    PaymentError, PaymentProvider, PaymentService, PaymentStatus, ProviderSession, Refund,
    RefundRequest, RefundResult, RefundStatus, VerifiedWebhook,
};
use tokio::sync::Barrier;

/// Minimal provider that counts calls and can hold a capture open so the claim
/// window is wide enough to overlap with the other racers.
struct TestProvider {
    id: String,
    capture_delay: Duration,
    capture_calls: AtomicUsize,
    refund_calls: AtomicUsize,
}

impl TestProvider {
    fn new(id: &str, capture_delay: Duration) -> Self {
        Self {
            id: id.to_owned(),
            capture_delay,
            capture_calls: AtomicUsize::new(0),
            refund_calls: AtomicUsize::new(0),
        }
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
        Ok(ProviderSession {
            provider_session_id: format!("{}_session", self.id),
            status: PaymentStatus::Pending,
            data: serde_json::json!({}),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        self.capture_calls.fetch_add(1, Ordering::SeqCst);
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

async fn setup(
    max_connections: u32,
    capture_delay: Duration,
) -> (
    tempfile::TempDir,
    StoreDatabase,
    Arc<PaymentService>,
    Arc<TestProvider>,
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
    let provider = Arc::new(TestProvider::new("concurrent", capture_delay));
    service
        .register_provider(provider.clone())
        .expect("register provider");
    (directory, db, Arc::new(service), provider)
}

async fn payment_count(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(db.pool())
        .await
        .expect("count payments")
}

async fn refund_rows(db: &StoreDatabase) -> Vec<(String, i64, String)> {
    sqlx::query_as("SELECT id, amount, status FROM refunds")
        .fetch_all(db.pool())
        .await
        .expect("read refunds")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_captures_produce_exactly_one_payment() {
    let (_directory, db, service, provider) = setup(8, Duration::from_millis(40)).await;
    let session = service
        .create_session(CreatePaymentSessionInput::new("concurrent", 1000, "usd"))
        .await
        .expect("create session");
    let session_id = session.id.to_string();

    let barrier = Arc::new(Barrier::new(6));
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let service = service.clone();
        let barrier = barrier.clone();
        let session_id = session_id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service.capture(&session_id).await
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
        "at least one capture must succeed: {failures:?}"
    );
    let payment_id = winners[0].id.clone();
    assert!(
        winners.iter().all(|payment| payment.id == payment_id),
        "every successful capture must return the same payment"
    );
    assert!(
        failures
            .iter()
            .all(|failure| matches!(failure, AppError::Conflict(_))),
        "a losing capture must fail with a conflict, got: {failures:?}"
    );
    assert_eq!(
        payment_count(&db).await,
        1,
        "six concurrent captures must persist exactly one payment"
    );
    assert_eq!(
        provider.capture_calls.load(Ordering::SeqCst),
        1,
        "the provider must be called once per capture attempt"
    );
    let payment = service.get_payment(payment_id.as_str()).await.unwrap();
    assert_eq!(payment.status, PaymentStatus::Captured);
    assert_eq!(payment.amount, 1000);
    assert_eq!(payment.currency_code, "USD");
    let stored = service.get_session(&session_id).await.unwrap();
    assert_eq!(stored.status, PaymentStatus::Captured);
    let captured_attempts = service
        .list_attempts(&session_id)
        .await
        .unwrap()
        .into_iter()
        .filter(|attempt| attempt.status == PaymentStatus::Captured)
        .count();
    assert_eq!(captured_attempts, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replaying_a_capture_after_the_race_returns_the_winner() {
    let (_directory, _db, service, provider) = setup(8, Duration::from_millis(30)).await;
    let session = service
        .create_session(CreatePaymentSessionInput::new("concurrent", 2500, "usd"))
        .await
        .expect("create session");
    let session_id = session.id.to_string();

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        let session_id = session_id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service.capture(&session_id).await
        }));
    }
    let mut raced = Vec::new();
    for task in tasks {
        raced.push(task.await.expect("capture task panicked"));
    }
    let winner = raced
        .iter()
        .find_map(|result| result.as_ref().ok())
        .expect("one racer must win")
        .clone();

    // After the race settles, a replay observes the captured state and must
    // return the same payment without another provider call.
    let replay = service.capture(&session_id).await.expect("replay capture");
    assert_eq!(replay.id, winner.id);
    assert_eq!(provider.capture_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_duplicate_refund_keys_produce_exactly_one_refund() {
    let (_directory, db, service, provider) = setup(8, Duration::ZERO).await;
    let session = service
        .create_session(CreatePaymentSessionInput::new("concurrent", 1000, "usd"))
        .await
        .expect("create session");
    let payment = service.capture(session.id.as_str()).await.unwrap();
    let payment_id = payment.id.to_string();

    let barrier = Arc::new(Barrier::new(4));
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let service = service.clone();
        let barrier = barrier.clone();
        let payment_id = payment_id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .refund_with_idempotency(
                    &payment_id,
                    400,
                    None,
                    Some("return-refund:race".to_owned()),
                )
                .await
        }));
    }

    let mut refunds: Vec<Refund> = Vec::new();
    let mut failures: Vec<AppError> = Vec::new();
    for task in tasks {
        match task.await.expect("refund task panicked") {
            Ok(refund) => refunds.push(refund),
            Err(error) => failures.push(error),
        }
    }

    assert!(
        failures.is_empty(),
        "a duplicated idempotency key must return the winning refund: {failures:?}"
    );
    let refund_id = refunds[0].id.clone();
    assert!(
        refunds.iter().all(|refund| refund.id == refund_id),
        "every caller with the same key must observe one refund row"
    );
    let rows = refund_rows(&db).await;
    assert_eq!(
        rows.len(),
        1,
        "duplicate refund keys must not add rows: {rows:?}"
    );
    let refunded_total: i64 = rows.iter().map(|(_, amount, _)| amount).sum();
    assert_eq!(refunded_total, 400, "the payment must be refunded once");
    assert!(provider.refund_calls.load(Ordering::SeqCst) >= 1);
    assert_eq!(
        service.get_payment(&payment_id).await.unwrap().status,
        PaymentStatus::PartiallyRefunded
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_naive_capture_records_two_payments_on_this_harness() {
    // Counterexample, deliberately asserting the bug: two connections both see
    // a pending session, both insert a payment and both mark it captured. The
    // assertions above only mean something because this harness can produce
    // that interleaving.
    let (_directory, db, service, _provider) = setup(2, Duration::ZERO).await;
    let session = service
        .create_session(CreatePaymentSessionInput::new("concurrent", 1000, "usd"))
        .await
        .expect("create session");
    let session_id = session.id.to_string();

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for index in 0..2 {
        let db = db.clone();
        let barrier = barrier.clone();
        let session_id = session_id.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = db.pool().acquire().await.unwrap();
            let status: String = sqlx::query_scalar("SELECT status FROM payment_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            barrier.wait().await;
            // Naive act: both callers saw a pending session.
            assert_eq!(status, "pending");
            sqlx::query(
                "INSERT INTO payments (id, provider, amount, currency_code, status, created_at, updated_at) \
                 VALUES (?, 'concurrent', 1000, 'USD', 'captured', 0, 0)",
            )
            .bind(format!("payment_naive_{index}"))
            .execute(&mut *connection)
            .await
            .unwrap();
            sqlx::query("UPDATE payment_sessions SET status = 'captured', updated_at = 0 WHERE id = ?")
                .bind(&session_id)
                .execute(&mut *connection)
                .await
                .unwrap();
            status
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    assert_eq!(
        payment_count(&db).await,
        2,
        "the harness reproduces a duplicated capture: two payments for one session"
    );
    assert_eq!(
        service.get_session(&session_id).await.unwrap().status,
        PaymentStatus::Captured
    );
}
