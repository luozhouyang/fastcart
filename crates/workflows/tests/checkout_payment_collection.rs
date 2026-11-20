//! Checkout wiring of payment collections.
//!
//! Opening a checkout opens the payment collection that represents the money
//! the checkout owes, and holds the session created for it. Completion keeps
//! the original single-session capture path, and only a collection that is
//! split across several sessions is captured as a collection.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_checkout::CheckoutStatus;
use fastcart_module_payment::{
    CaptureRequest, CaptureResult, CreatePaymentSessionInput, CreateProviderSession,
    PaymentCollectionStatus, PaymentError, PaymentProvider, PaymentService,
    PaymentStatus as PaymentState, ProviderRegistry, ProviderSession, RefundRequest, RefundResult,
    RefundStatus, VerifiedWebhook,
};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

/// Provider that records the idempotency key of every capture, so a test can
/// pin the exact provider calls the checkout makes.
struct RecordingProvider {
    id: String,
    sessions: AtomicUsize,
    refunds: AtomicUsize,
    captures: std::sync::Mutex<Vec<RecordedCapture>>,
}

#[derive(Clone, Debug)]
struct RecordedCapture {
    provider_session_id: String,
    amount: i64,
    idempotency_key: Option<String>,
}

impl RecordingProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            sessions: AtomicUsize::new(0),
            refunds: AtomicUsize::new(0),
            captures: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn captures(&self) -> Vec<RecordedCapture> {
        self.captures.lock().unwrap().clone()
    }
}

#[async_trait]
impl PaymentProvider for RecordingProvider {
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
            status: PaymentState::Pending,
            data: serde_json::json!({}),
        })
    }

    async fn capture(&self, request: CaptureRequest) -> Result<CaptureResult, PaymentError> {
        let provider_session_id = request.provider_session_id.clone();
        self.captures.lock().unwrap().push(RecordedCapture {
            provider_session_id: provider_session_id.clone(),
            amount: request.amount,
            idempotency_key: request.idempotency_key,
        });
        Ok(CaptureResult {
            provider_payment_id: format!("{}_payment_{provider_session_id}", self.id),
            amount: request.amount,
            currency_code: request.currency_code,
            status: PaymentState::Captured,
        })
    }

    async fn refund(&self, request: RefundRequest) -> Result<RefundResult, PaymentError> {
        let index = self.refunds.fetch_add(1, Ordering::SeqCst);
        Ok(RefundResult {
            provider_refund_id: format!("{}_refund_{index}", self.id),
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

/// Catalog, price and shipping for a single-variant cart of `amount` minor
/// units, plus the cart itself.
async fn cart_for(db: &StoreDatabase, amount: i64) -> (Cart, CartContext) {
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Collection test product".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, usd(), amount))
        .await
        .unwrap();

    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    let option = shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Standard shipping".to_owned(),
            amount_minor: 0,
            currency: usd(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: usd().to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap();
    let cart = carts
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.to_string()),
            1,
            &context,
        )
        .await
        .unwrap();
    let cart = carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();
    (cart, context)
}

fn usd() -> fastcart_kernel::CurrencyCode {
    fastcart_kernel::CurrencyCode::new("USD").unwrap()
}

async fn count_rows(db: &StoreDatabase, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db.pool())
        .await
        .expect("count rows")
}

#[tokio::test]
async fn a_checkout_opens_a_collection_that_holds_its_payment_session() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let (cart, context) = cart_for(&db, 1_500).await;

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(RecordingProvider::new("recording")))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "recording",
            context,
        ))
        .await
        .unwrap();
    let session_id = checkout.payment_session_id.clone().unwrap();

    let collection = payments
        .collection_for_checkout(checkout.id.as_str())
        .await
        .unwrap()
        .expect("a checkout opens a payment collection");
    assert_eq!(collection.amount, 1_500);
    assert_eq!(collection.currency_code, "USD");
    let sessions = payments
        .list_collection_sessions(collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session_id);
    assert_eq!(
        payments
            .collection_status(collection.id.as_str())
            .await
            .unwrap(),
        PaymentCollectionStatus::Open
    );
    assert_eq!(count_rows(&db, "payment_collections").await, 1);
}

#[tokio::test]
async fn a_single_session_checkout_captures_with_the_legacy_idempotency_key() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let (cart, context) = cart_for(&db, 1_500).await;

    let providers = ProviderRegistry::new();
    let provider = Arc::new(RecordingProvider::new("recording"));
    providers.register(provider.clone()).unwrap();
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "recording",
            context.clone(),
        ))
        .await
        .unwrap();
    let session_id = checkout.payment_session_id.clone().unwrap();
    let completed = workflow.complete(&checkout.id, &context).await.unwrap();

    assert_eq!(completed.payment.status, PaymentState::Captured);
    assert_eq!(completed.payment.amount, 1_500);
    let captures = provider.captures();
    assert_eq!(captures.len(), 1, "a single-session checkout captures once");
    assert_eq!(captures[0].amount, 1_500);
    assert_eq!(
        captures[0].idempotency_key.as_deref(),
        Some(format!("capture:{session_id}").as_str()),
        "the single-session path keeps its historical provider idempotency key"
    );
    assert_eq!(
        count_rows(&db, "payments").await,
        1,
        "a single-session checkout records one payment"
    );
}

/// Characterisation test of the historical single-session path.
///
/// The capture path of a checkout that pays with one session is the one every
/// existing checkout takes, so it is kept exactly as it was: it captures the
/// session and never consults the collection. The only state in which the two
/// can be told apart is a session that does not cover its collection, which no
/// API can produce — the test arranges it directly to pin that no coverage
/// check was added to the legacy path. Capturing more than the session holds
/// (or refusing to capture it) are both behaviour changes for existing
/// checkouts; the collection instead records what was actually captured.
#[tokio::test]
async fn a_single_session_checkout_still_captures_only_its_session() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let (cart, context) = cart_for(&db, 1_500).await;

    let providers = ProviderRegistry::new();
    let provider = Arc::new(RecordingProvider::new("recording"));
    providers.register(provider.clone()).unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "recording",
            context.clone(),
        ))
        .await
        .unwrap();
    let session_id = checkout.payment_session_id.clone().unwrap();
    sqlx::query("UPDATE payment_sessions SET amount = ? WHERE id = ?")
        .bind(400)
        .bind(session_id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();

    assert_eq!(completed.payment.amount, 400);
    let captures = provider.captures();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].amount, 400);
    assert_eq!(
        captures[0].idempotency_key.as_deref(),
        Some(format!("capture:{session_id}").as_str())
    );
    let collection = payments
        .collection_for_checkout(checkout.id.as_str())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        payments
            .collection_status(collection.id.as_str())
            .await
            .unwrap(),
        PaymentCollectionStatus::PartiallyCaptured,
        "what was captured is visible on the collection instead of failing the checkout"
    );
}

#[tokio::test]
async fn a_combination_payment_places_one_order_for_the_whole_cart() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let (cart, context) = cart_for(&db, 1_500).await;

    let providers = ProviderRegistry::new();
    let provider = Arc::new(RecordingProvider::new("recording"));
    providers.register(provider.clone()).unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "recording",
            context.clone(),
        ))
        .await
        .unwrap();
    let checkout_session = checkout.payment_session_id.clone().unwrap();

    // No API splits a checkout payment yet, so the split is arranged directly:
    // the checkout's own session is re-priced and a second session pays the
    // rest of the collection.
    sqlx::query("UPDATE payment_sessions SET amount = ? WHERE id = ?")
        .bind(400)
        .bind(checkout_session.as_str())
        .execute(db.pool())
        .await
        .unwrap();
    let mut second_session = CreatePaymentSessionInput::new("recording", 1_100, "usd");
    second_session.cart_id = Some(cart.id.clone());
    second_session.checkout_id = Some(checkout.id.clone());
    let second_session = payments.create_session(second_session).await.unwrap();
    let collection = payments
        .collection_for_checkout(checkout.id.as_str())
        .await
        .unwrap()
        .unwrap();
    payments
        .attach_session_to_collection(collection.id.as_str(), second_session.id.as_str())
        .await
        .unwrap();

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();

    assert_eq!(completed.checkout.status, CheckoutStatus::OrderPlaced);
    assert_eq!(
        completed.order.total, 1_500,
        "the order records the whole cart, not one session"
    );
    assert_eq!(
        completed.order.payment_status,
        fastcart_module_order::PaymentStatus::Captured
    );
    let captures = provider.captures();
    assert_eq!(
        captures
            .iter()
            .map(|capture| capture.amount)
            .collect::<Vec<_>>(),
        vec![400, 1_100],
        "every session of the collection is captured once"
    );
    let expected_sessions: Vec<String> = vec![
        payments
            .get_session(checkout_session.as_str())
            .await
            .unwrap()
            .provider_session_id
            .unwrap(),
        payments
            .get_session(second_session.id.as_str())
            .await
            .unwrap()
            .provider_session_id
            .unwrap(),
    ];
    assert_eq!(
        captures
            .iter()
            .map(|capture| capture.provider_session_id.clone())
            .collect::<Vec<_>>(),
        expected_sessions,
        "the captures belong to the checkout's session and the split-off session"
    );
    let rows: Vec<(String, Option<String>, i64, String)> =
        sqlx::query_as("SELECT id, order_id, amount, status FROM payments ORDER BY amount")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter().map(|row| row.2).sum::<i64>(),
        1_500,
        "the captures add up to the order total"
    );
    assert!(
        rows.iter()
            .all(|row| row.1.as_deref() == Some(completed.order.id.as_str())
                && row.3 == "captured"),
        "every captured payment must be assigned to the order: {rows:?}"
    );
    for session in [&checkout_session, &second_session.id] {
        assert_eq!(
            payments.get_session(session.as_str()).await.unwrap().status,
            PaymentState::Captured
        );
    }
    assert_eq!(
        payments
            .collection_status(collection.id.as_str())
            .await
            .unwrap(),
        PaymentCollectionStatus::Captured
    );
    assert_eq!(
        completed.payment.id,
        payments
            .payment_for_session_id(checkout_session.as_str())
            .await
            .unwrap()
            .id,
        "the result reports the payment of the checkout's own session"
    );

    // A replay returns the placed order without capturing anything again.
    let replay = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(replay.order.id, completed.order.id);
    assert_eq!(replay.payment.id, completed.payment.id);
    assert_eq!(provider.captures().len(), 2);
    assert_eq!(count_rows(&db, "payments").await, 2);
}
