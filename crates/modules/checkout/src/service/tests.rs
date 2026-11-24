use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CartId, CheckoutId, OrderId, PaymentSessionId, StoreId, now_timestamp};
use serde_json::json;

use super::CheckoutService;
use crate::domain::{CheckoutStatus, CreateCheckoutInput, UpdateCheckoutDetailsInput};

/// Test wrapper that seeds the cart rows `create` now validates.
struct TestService {
    service: CheckoutService,
}

impl std::ops::Deref for TestService {
    type Target = CheckoutService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

impl TestService {
    async fn create(
        &self,
        input: CreateCheckoutInput,
    ) -> Result<crate::domain::CheckoutSession, fastcart_kernel::AppError> {
        seed_cart(&self.service, &input.cart_id).await;
        self.service.create(input).await
    }
}

async fn seed_cart(service: &CheckoutService, cart: &CartId) {
    let now = now_timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO carts
                (id, currency_code, status, created_at, updated_at)
             VALUES (?, 'USD', 'open', ?, ?)",
    )
    .bind(cart.as_str())
    .bind(now)
    .bind(now)
    .execute(service.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO cart_items
                (id, cart_id, product_id, variant_id, quantity, title, unit_price,
                 currency_code, created_at, updated_at)
             VALUES (?, ?, 'prod_test', 'variant_test', 1, 'Test', 2500, 'USD', ?, ?)",
    )
    .bind(format!("item_{}", cart.as_str()))
    .bind(cart.as_str())
    .bind(now)
    .bind(now)
    .execute(service.db.pool())
    .await
    .unwrap();
}

async fn service() -> TestService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    TestService {
        service: CheckoutService::new(db),
    }
}

fn input(cart: CartId) -> CreateCheckoutInput {
    CreateCheckoutInput {
        cart_id: cart,
        customer_id: None,
        email: Some("buyer@example.com".to_owned()),
        currency_code: "USD".to_owned(),
        region_id: None,
        shipping_address: json!({"city": "Berlin"}),
        billing_address: json!({"city": "Berlin"}),
        shipping_method: json!({"name": "Standard"}),
        totals: json!({"total": 2500}),
        promotion_codes: vec!["SAVE10".to_owned()],
        idempotency_key: None,
        expires_in_seconds: 3_600,
    }
}

#[tokio::test]
async fn creates_session_and_history() {
    let service = service().await;
    let cart = CartId::new();
    let session = service.create(input(cart.clone())).await.unwrap();

    assert_eq!(session.status, CheckoutStatus::Created);
    assert_eq!(session.cart_id, cart);
    assert_eq!(session.currency_code, "USD");
    assert_eq!(session.promotion_codes, vec!["SAVE10".to_owned()]);
    assert!(session.expires_at.is_some());

    let history = service.list_history(&session.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].to_state, CheckoutStatus::Created);
    assert_eq!(history[0].from_state, None);
}

#[tokio::test]
async fn persists_the_region_id() {
    let service = service().await;
    let cart = CartId::new();
    let mut request = input(cart);
    request.region_id = Some("reg_test".to_owned());

    let session = service.create(request).await.unwrap();
    assert_eq!(session.region_id.as_deref(), Some("reg_test"));

    let loaded = service.get(&session.id).await.unwrap();
    assert_eq!(loaded.region_id.as_deref(), Some("reg_test"));
}

#[tokio::test]
async fn create_is_idempotent_by_key() {
    let service = service().await;
    let cart = CartId::new();
    let mut request = input(cart);
    request.idempotency_key = Some("checkout-key".to_owned());

    let first = service.create(request.clone()).await.unwrap();
    let second = service.create(request).await.unwrap();

    assert_eq!(first.id, second.id);
    let found = service
        .get_by_idempotency_key("checkout-key")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, first.id);
}

#[tokio::test]
async fn rejects_a_second_session_for_the_same_cart() {
    let service = service().await;
    let cart = CartId::new();
    service.create(input(cart.clone())).await.unwrap();

    let error = service.create(input(cart)).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn accepts_the_documented_transition_chain() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();

    service
        .transition(&session.id, CheckoutStatus::Validating, Some("validating"))
        .await
        .unwrap();
    let ready = service
        .transition(&session.id, CheckoutStatus::ReadyForPayment, Some("ready"))
        .await
        .unwrap();
    assert_eq!(ready.status, CheckoutStatus::ReadyForPayment);

    let payment_session = PaymentSessionId::new();
    let pending = service
        .attach_payment_session(&session.id, &payment_session)
        .await
        .unwrap();
    assert_eq!(pending.status, CheckoutStatus::PaymentPending);
    assert_eq!(pending.payment_session_id, Some(payment_session));

    service
        .transition(&session.id, CheckoutStatus::PaymentConfirmed, None)
        .await
        .unwrap();
    let order = OrderId::new();
    let placed = service
        .mark_order_placed(&session.id, &order)
        .await
        .unwrap();
    assert_eq!(placed.status, CheckoutStatus::OrderPlaced);
    assert_eq!(placed.order_id, Some(order));

    let history = service.list_history(&session.id).await.unwrap();
    assert_eq!(history.len(), 6);
    assert_eq!(
        history.last().unwrap().to_state,
        CheckoutStatus::OrderPlaced
    );
}

#[tokio::test]
async fn rejects_illegal_transitions() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();

    let error = service
        .transition(&session.id, CheckoutStatus::PaymentConfirmed, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    service
        .transition(&session.id, CheckoutStatus::Failed, Some("bad data"))
        .await
        .unwrap();
    let terminal = service
        .transition(&session.id, CheckoutStatus::Validating, None)
        .await
        .unwrap_err();
    assert!(matches!(terminal, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn payment_session_requires_ready_state() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();

    let error = service
        .attach_payment_session(&session.id, &PaymentSessionId::new())
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn order_placement_requires_confirmed_payment() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();

    let error = service
        .mark_order_placed(&session.id, &OrderId::new())
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn updates_details_only_in_editable_states() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();

    let updated = service
        .update_details(
            &session.id,
            UpdateCheckoutDetailsInput {
                email: Some("new@example.com".to_owned()),
                totals: Some(json!({"total": 9000})),
                promotion_codes: Some(vec!["NEW".to_owned()]),
                ..UpdateCheckoutDetailsInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.email.as_deref(), Some("new@example.com"));
    assert_eq!(updated.totals, json!({"total": 9000}));
    assert_eq!(updated.promotion_codes, vec!["NEW".to_owned()]);

    service
        .transition(&session.id, CheckoutStatus::Validating, None)
        .await
        .unwrap();
    service
        .transition(&session.id, CheckoutStatus::ReadyForPayment, None)
        .await
        .unwrap();
    service
        .transition(&session.id, CheckoutStatus::PaymentPending, None)
        .await
        .unwrap();

    let error = service
        .update_details(
            &session.id,
            UpdateCheckoutDetailsInput {
                email: Some("other@example.com".to_owned()),
                ..UpdateCheckoutDetailsInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn expires_due_sessions() {
    let service = service().await;
    let expiring = service.create(input(CartId::new())).await.unwrap();
    let mut long_lived_input = input(CartId::new());
    long_lived_input.expires_in_seconds = 86_400;
    let long_lived = service.create(long_lived_input).await.unwrap();

    let count = service.expire_due(now_timestamp() + 7_200).await.unwrap();
    assert_eq!(count, 1);

    let expired = service.get(&expiring.id).await.unwrap();
    assert_eq!(expired.status, CheckoutStatus::Failed);
    let history = service.list_history(&expired.id).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].reason.as_deref(), Some("expired"));

    let still_open = service.get(&long_lived.id).await.unwrap();
    assert_eq!(still_open.status, CheckoutStatus::Created);
}

#[tokio::test]
async fn expire_due_keeps_checkouts_with_captured_payments() {
    let service = service().await;
    let captured = service.create(input(CartId::new())).await.unwrap();
    let pending = service.create(input(CartId::new())).await.unwrap();
    for session in [&captured, &pending] {
        for status in [
            CheckoutStatus::Validating,
            CheckoutStatus::ReadyForPayment,
            CheckoutStatus::PaymentPending,
        ] {
            service.transition(&session.id, status, None).await.unwrap();
        }
    }

    let now = now_timestamp();
    let payment_session_id = "pay_captured";
    sqlx::query(
        "INSERT INTO payment_sessions
                (id, checkout_id, provider, amount, currency_code, status, created_at, updated_at)
             VALUES (?, ?, 'manual', 2500, 'USD', 'captured', ?, ?)",
    )
    .bind(payment_session_id)
    .bind(captured.id.as_str())
    .bind(now)
    .bind(now)
    .execute(service.db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE checkout_sessions SET payment_session_id = ? WHERE id = ?")
        .bind(payment_session_id)
        .bind(captured.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let expired = service.expire_due(now + 7_200).await.unwrap();
    assert_eq!(expired, 1);
    // Captured money must stay completable rather than being failed.
    assert_eq!(
        service.get(&captured.id).await.unwrap().status,
        CheckoutStatus::PaymentPending
    );
    assert_eq!(
        service.get(&pending.id).await.unwrap().status,
        CheckoutStatus::Failed
    );
}

#[tokio::test]
async fn validates_currency_email_and_expiry() {
    let service = service().await;

    let mut request = input(CartId::new());
    request.currency_code = "US".to_owned();
    assert!(service.create(request).await.is_err());

    let mut request = input(CartId::new());
    request.email = Some("not-an-email".to_owned());
    assert!(service.create(request).await.is_err());

    let mut request = input(CartId::new());
    request.expires_in_seconds = 0;
    assert!(service.create(request).await.is_err());
}

#[tokio::test]
async fn missing_session_is_not_found() {
    let service = service().await;
    let error = service.get(&CheckoutId::new()).await.unwrap_err();
    assert!(error.is_not_found());
    assert!(service.get_by_cart(&CartId::new()).await.unwrap().is_none());
}

#[tokio::test]
async fn snapshots_cart_items_at_creation() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();
    assert_eq!(
        session.cart_items,
        vec![crate::domain::CheckoutCartItem {
            variant_id: "variant_test".to_owned(),
            quantity: 1,
        }]
    );
}

#[tokio::test]
async fn failed_session_is_superseded_on_retry() {
    let service = service().await;
    let mut request = input(CartId::new());
    request.idempotency_key = Some("retry-key".to_owned());
    let first = service.create(request.clone()).await.unwrap();
    service
        .transition(&first.id, CheckoutStatus::Failed, Some("transient"))
        .await
        .unwrap();

    let second = service.create(request).await.unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(second.status, CheckoutStatus::Created);
}

#[tokio::test]
async fn placement_claim_fences_concurrent_completion() {
    let service = service().await;
    let session = service.create(input(CartId::new())).await.unwrap();
    for status in [
        CheckoutStatus::Validating,
        CheckoutStatus::ReadyForPayment,
        CheckoutStatus::PaymentPending,
    ] {
        service.transition(&session.id, status, None).await.unwrap();
    }

    let now = now_timestamp();
    let first = service
        .claim_placement(&session.id, now, 120)
        .await
        .unwrap();
    assert!(first.is_some());
    // A second runner at the same instant loses the race.
    assert!(
        service
            .claim_placement(&session.id, now, 120)
            .await
            .unwrap()
            .is_none()
    );

    // After the lease expires a retry can take over.
    let second = service
        .claim_placement(&session.id, now + 121, 120)
        .await
        .unwrap();
    assert!(second.is_some());
    let token: Option<String> =
        sqlx::query_scalar("SELECT placement_token FROM checkout_sessions WHERE id = ?")
            .bind(session.id.as_str())
            .fetch_one(service.db.pool())
            .await
            .unwrap();
    assert_eq!(token, second);

    // A stale owner cannot release the new owner's claim.
    service
        .release_placement(&session.id, first.as_deref().unwrap())
        .await
        .unwrap();
    let token: Option<String> =
        sqlx::query_scalar("SELECT placement_token FROM checkout_sessions WHERE id = ?")
            .bind(session.id.as_str())
            .fetch_one(service.db.pool())
            .await
            .unwrap();
    assert_eq!(token, second);

    // The current owner can release, and the checkout becomes reclaimable.
    service
        .release_placement(&session.id, second.as_deref().unwrap())
        .await
        .unwrap();
    assert!(
        service
            .claim_placement(&session.id, now + 122, 120)
            .await
            .unwrap()
            .is_some()
    );

    // Placing the order clears the claim so it cannot fence a replay.
    service
        .transition(&session.id, CheckoutStatus::PaymentConfirmed, None)
        .await
        .unwrap();
    service
        .mark_order_placed(&session.id, &OrderId::new())
        .await
        .unwrap();
    let token: Option<String> =
        sqlx::query_scalar("SELECT placement_token FROM checkout_sessions WHERE id = ?")
            .bind(session.id.as_str())
            .fetch_one(service.db.pool())
            .await
            .unwrap();
    assert!(token.is_none());
    assert!(
        service
            .claim_placement(&session.id, now + 200, 120)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn rejects_empty_completed_or_mismatched_carts() {
    let service = service().await;

    // Missing cart.
    let error = service
        .service
        .create(input(CartId::new()))
        .await
        .unwrap_err();
    assert!(error.is_not_found());

    // Completed cart.
    let cart = CartId::new();
    seed_cart(&service.service, &cart).await;
    sqlx::query("UPDATE carts SET status = 'completed' WHERE id = ?")
        .bind(cart.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();
    let error = service.service.create(input(cart)).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    // Currency mismatch.
    let cart = CartId::new();
    let mut request = input(cart);
    request.currency_code = "EUR".to_owned();
    let error = service.create(request).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}
