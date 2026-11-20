//! The order side of a payment collection.
//!
//! A collection starts its life in a checkout and ends it as the money of an
//! order:
//!
//! * `assign_collection_to_order` links a collection to the order it paid for,
//!   idempotently and with one collection per order,
//! * `create_collection_for_order` opens the collection of an order that has
//!   none, idempotently by the order,
//! * `split_collection_session` moves part of the money a collection's own
//!   session holds onto a second session, which is what a combination payment
//!   needs: the sessions of a collection must keep covering its amount exactly,
//!   or `capture_collection` refuses to settle it, and concurrent splits cannot
//!   spend the same money twice.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, OrderId, StoreId};
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession,
    PaymentCollection, PaymentCollectionStatus, PaymentError, PaymentProvider, PaymentService,
    PaymentSession, PaymentStatus, ProviderSession, RefundRequest, RefundResult, RefundStatus,
    VerifiedWebhook,
};
use tokio::sync::Barrier;

/// Minimal provider that counts the sessions it was asked to create.
struct CountingProvider {
    id: String,
    sessions: AtomicUsize,
    captures: AtomicUsize,
}

impl CountingProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            sessions: AtomicUsize::new(0),
            captures: AtomicUsize::new(0),
        }
    }

    fn session_count(&self) -> usize {
        self.sessions.load(Ordering::SeqCst)
    }

    fn capture_count(&self) -> usize {
        self.captures.load(Ordering::SeqCst)
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
        self.captures.fetch_add(1, Ordering::SeqCst);
        Ok(CaptureResult {
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

async fn count_rows(db: &StoreDatabase, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db.pool())
        .await
        .expect("count rows")
}

async fn session_amount(db: &StoreDatabase, session_id: &str) -> i64 {
    sqlx::query_scalar("SELECT amount FROM payment_sessions WHERE id = ?")
        .bind(session_id)
        .fetch_one(db.pool())
        .await
        .expect("session amount")
}

/// A collection of `amount` USD holding one session of the same amount, the
/// shape a checkout opens.
async fn collection_with_primary(
    service: &PaymentService,
    checkout_id: &str,
    amount: i64,
) -> (PaymentCollection, PaymentSession) {
    let collection = service
        .create_collection(checkout_id, amount, "usd")
        .await
        .expect("create collection");
    let session = service
        .create_session(CreatePaymentSessionInput::new("collections", amount, "usd"))
        .await
        .expect("create primary session");
    service
        .attach_session_to_collection(collection.id.as_str(), session.id.as_str())
        .await
        .expect("attach primary session");
    (collection, session)
}

#[tokio::test]
async fn assigning_a_collection_to_an_order_is_idempotent() {
    let (service, db, _provider) = setup().await;
    let (collection, _session) = collection_with_primary(&service, "checkout-1", 1_000).await;
    let order_id = OrderId::new();

    let assigned = service
        .assign_collection_to_order(collection.id.as_str(), &order_id)
        .await
        .unwrap();
    assert_eq!(assigned.order_id.as_ref(), Some(&order_id));
    assert_eq!(assigned.id, collection.id);
    assert_eq!(
        assigned.checkout_id, collection.checkout_id,
        "the checkout the collection was opened for is kept"
    );

    let replay = service
        .assign_collection_to_order(collection.id.as_str(), &order_id)
        .await
        .unwrap();
    assert_eq!(replay.order_id.as_ref(), Some(&order_id));
    assert_eq!(
        count_rows(&db, "payment_collections").await,
        1,
        "assigning again must not open another collection"
    );
    assert_eq!(
        service
            .get_collection(collection.id.as_str())
            .await
            .unwrap()
            .order_id,
        Some(order_id)
    );
}

#[tokio::test]
async fn a_collection_cannot_be_assigned_to_a_second_order() {
    let (service, db, _provider) = setup().await;
    let (collection, _session) = collection_with_primary(&service, "checkout-1", 1_000).await;
    let first = OrderId::new();
    let second = OrderId::new();
    service
        .assign_collection_to_order(collection.id.as_str(), &first)
        .await
        .unwrap();

    let error = service
        .assign_collection_to_order(collection.id.as_str(), &second)
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(
        service
            .get_collection(collection.id.as_str())
            .await
            .unwrap()
            .order_id,
        Some(first),
        "the first order keeps the collection"
    );
    assert_eq!(count_rows(&db, "payment_collections").await, 1);
}

#[tokio::test]
async fn an_order_keeps_the_one_collection_it_was_given() {
    let (service, _db, _provider) = setup().await;
    let (first, _session) = collection_with_primary(&service, "checkout-1", 1_000).await;
    let (second, _other) = collection_with_primary(&service, "checkout-2", 1_000).await;
    let order_id = OrderId::new();

    service
        .assign_collection_to_order(first.id.as_str(), &order_id)
        .await
        .unwrap();
    let error = service
        .assign_collection_to_order(second.id.as_str(), &order_id)
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(
        service
            .get_collection(second.id.as_str())
            .await
            .unwrap()
            .order_id,
        None,
        "the rejected collection keeps no order"
    );
}

#[tokio::test]
async fn assigning_an_unknown_collection_is_not_found() {
    let (service, _db, _provider) = setup().await;

    let error = service
        .assign_collection_to_order("pcoll_missing", &OrderId::new())
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::NotFound(_)), "{error:?}");
}

#[tokio::test]
async fn creating_a_collection_for_an_order_is_idempotent() {
    let (service, db, _provider) = setup().await;
    let order_id = OrderId::new();

    let created = service
        .create_collection_for_order(&order_id, 500, "usd")
        .await
        .unwrap();

    assert_eq!(created.order_id.as_ref(), Some(&order_id));
    assert_eq!(created.checkout_id, None);
    assert_eq!(created.amount, 500);
    assert_eq!(created.currency_code, "USD");
    assert_eq!(created.status, PaymentCollectionStatus::Open);

    let replay = service
        .create_collection_for_order(&order_id, 500, "USD")
        .await
        .unwrap();
    assert_eq!(replay.id, created.id, "an order has one collection");
    assert_eq!(count_rows(&db, "payment_collections").await, 1);

    for error in [
        service
            .create_collection_for_order(&order_id, 400, "usd")
            .await
            .unwrap_err(),
        service
            .create_collection_for_order(&order_id, 500, "eur")
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    }
    assert_eq!(count_rows(&db, "payment_collections").await, 1);

    // A second order gets its own collection.
    let other = service
        .create_collection_for_order(&OrderId::new(), 500, "usd")
        .await
        .unwrap();
    assert_ne!(other.id, created.id);
}

#[tokio::test]
async fn creating_a_collection_for_an_order_validates_its_amount_and_currency() {
    let (service, db, _provider) = setup().await;

    assert!(matches!(
        service
            .create_collection_for_order(&OrderId::new(), -1, "usd")
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
    assert!(matches!(
        service
            .create_collection_for_order(&OrderId::new(), 500, "usdollars")
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
    assert_eq!(count_rows(&db, "payment_collections").await, 0);
}

#[tokio::test]
async fn a_placed_order_returns_the_collection_of_its_checkout() {
    let (service, _db, _provider) = setup().await;
    let (collection, _session) = collection_with_primary(&service, "checkout-1", 1_000).await;
    let order_id = OrderId::new();
    service
        .assign_collection_to_order(collection.id.as_str(), &order_id)
        .await
        .unwrap();

    // The order already holds the money its checkout opened a collection for,
    // so opening one for the order must converge on that collection.
    let replay = service
        .create_collection_for_order(&order_id, 1_000, "USD")
        .await
        .unwrap();
    assert_eq!(replay.id, collection.id);
    assert_eq!(
        replay.checkout_id, collection.checkout_id,
        "the checkout link survives the replay"
    );

    // An additional charge of another amount cannot be that collection.
    let error = service
        .create_collection_for_order(&order_id, 500, "USD")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
}

#[tokio::test]
async fn splitting_a_collection_session_moves_money_onto_a_new_session() {
    let (service, db, provider) = setup().await;
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;

    let mut split = CreatePaymentSessionInput::new("collections", 400, "USD");
    split.checkout_id = Some(fastcart_kernel::CheckoutId::from_string("checkout-1"));
    let created = service
        .split_collection_session(collection.id.as_str(), primary.id.as_str(), split)
        .await
        .unwrap();

    assert_eq!(created.amount, 400);
    assert_eq!(created.currency_code, "USD");
    assert_eq!(
        created.checkout_id.as_ref().map(ToString::to_string),
        Some("checkout-1".to_owned())
    );
    // The money came out of the session the collection already held, so the
    // sessions still cover the collection exactly.
    assert_eq!(session_amount(&db, primary.id.as_str()).await, 1_100);
    let sessions = service
        .list_collection_sessions(collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions.iter().map(|session| session.amount).sum::<i64>(),
        1_500,
        "a split never changes what the collection's sessions cover"
    );
    assert_eq!(
        provider.session_count(),
        2,
        "one provider session for the split-off session"
    );
    assert_eq!(
        provider.capture_count(),
        0,
        "splitting a payment must not charge anything yet"
    );

    // The split payment settles the collection through the normal path.
    let payments = service
        .capture_collection(collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(
        payments
            .iter()
            .map(|payment| payment.amount)
            .collect::<Vec<_>>(),
        vec![1_100, 400]
    );
    assert_eq!(
        service
            .collection_status(collection.id.as_str())
            .await
            .unwrap(),
        PaymentCollectionStatus::Captured
    );
}

#[tokio::test]
async fn a_split_that_outgrows_the_primary_session_is_rejected() {
    let (service, db, provider) = setup().await;
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;

    for amount in [1_500, 1_501] {
        let error = service
            .split_collection_session(
                collection.id.as_str(),
                primary.id.as_str(),
                CreatePaymentSessionInput::new("collections", amount, "USD"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    }

    assert_eq!(session_amount(&db, primary.id.as_str()).await, 1_500);
    assert_eq!(count_rows(&db, "payment_sessions").await, 1);
    assert_eq!(
        provider.session_count(),
        1,
        "a rejected split must not open a provider session"
    );
}

#[tokio::test]
async fn a_split_must_be_positive_and_in_the_collection_currency() {
    let (service, db, _provider) = setup().await;
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;

    for input in [
        CreatePaymentSessionInput::new("collections", 0, "USD"),
        CreatePaymentSessionInput::new("collections", -400, "USD"),
    ] {
        assert!(matches!(
            service
                .split_collection_session(collection.id.as_str(), primary.id.as_str(), input)
                .await
                .unwrap_err(),
            AppError::Validation(_)
        ));
    }
    assert!(matches!(
        service
            .split_collection_session(
                collection.id.as_str(),
                primary.id.as_str(),
                CreatePaymentSessionInput::new("collections", 400, "EUR"),
            )
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));

    assert_eq!(session_amount(&db, primary.id.as_str()).await, 1_500);
    assert_eq!(count_rows(&db, "payment_sessions").await, 1);
}

#[tokio::test]
async fn a_split_takes_the_session_of_the_collection() {
    let (service, db, _provider) = setup().await;
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;
    let (other, other_primary) = collection_with_primary(&service, "checkout-2", 1_500).await;

    let error = service
        .split_collection_session(
            other.id.as_str(),
            primary.id.as_str(),
            CreatePaymentSessionInput::new("collections", 400, "USD"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");

    // The collection's own session is the one a split may spend.
    let split = service
        .split_collection_session(
            collection.id.as_str(),
            primary.id.as_str(),
            CreatePaymentSessionInput::new("collections", 1_400, "USD"),
        )
        .await
        .unwrap();
    assert_eq!(split.amount, 1_400);

    // Only the money the session still holds is left to split.
    let error = service
        .split_collection_session(
            collection.id.as_str(),
            primary.id.as_str(),
            CreatePaymentSessionInput::new("collections", 200, "USD"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(
        service
            .list_collection_sessions(other.id.as_str())
            .await
            .unwrap()
            .len(),
        1,
        "a rejected split must not touch another collection"
    );
    assert_eq!(session_amount(&db, other_primary.id.as_str()).await, 1_500);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_splits_never_spend_the_same_money_twice() {
    let directory = tempfile::tempdir().expect("temp dir");
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        8,
    )
    .await
    .expect("multi-connection store database");
    let service = Arc::new(PaymentService::new(db.clone()));
    service
        .register_provider(Arc::new(CountingProvider::new("collections")))
        .expect("register provider");
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;

    // Both callers want more than half of the money the primary session holds,
    // so only one of them can be served.
    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        let collection_id = collection.id.as_str().to_owned();
        let primary_id = primary.id.as_str().to_owned();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .split_collection_session(
                    &collection_id,
                    &primary_id,
                    CreatePaymentSessionInput::new("collections", 1_000, "USD"),
                )
                .await
        }));
    }

    let mut winners = Vec::new();
    let mut losers = Vec::new();
    for task in tasks {
        match task.await.expect("split task panicked") {
            Ok(session) => winners.push(session),
            Err(error) => losers.push(error),
        }
    }

    assert_eq!(
        winners.len(),
        1,
        "only one split may spend the remainder: {losers:?}"
    );
    assert!(
        losers
            .iter()
            .all(|error| matches!(error, AppError::Conflict(_))),
        "a losing split must fail cleanly: {losers:?}"
    );
    assert_eq!(winners[0].amount, 1_000);
    let sessions = service
        .list_collection_sessions(collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions.iter().map(|session| session.amount).sum::<i64>(),
        1_500,
        "the collection is covered exactly, whatever the race"
    );
    assert_eq!(session_amount(&db, primary.id.as_str()).await, 500);
}

#[tokio::test]
async fn a_settled_session_cannot_be_split() {
    let (service, _db, _provider) = setup().await;
    let (collection, primary) = collection_with_primary(&service, "checkout-1", 1_500).await;
    service.capture(primary.id.as_str()).await.unwrap();

    let error = service
        .split_collection_session(
            collection.id.as_str(),
            primary.id.as_str(),
            CreatePaymentSessionInput::new("collections", 400, "USD"),
        )
        .await
        .unwrap_err();

    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(
        service
            .list_collection_sessions(collection.id.as_str())
            .await
            .unwrap()
            .len(),
        1,
        "money that already moved cannot be split again"
    );
}
