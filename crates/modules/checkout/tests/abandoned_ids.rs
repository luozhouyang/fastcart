//! Maintenance decides which promotion claims to give back by asking the
//! checkout module which of the checkouts holding a claim can never complete.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CartId, CheckoutId, StoreId};
use fastcart_module_checkout::CheckoutService;

async fn session(db: &StoreDatabase, status: &str) -> CheckoutId {
    let id = CheckoutId::new();
    sqlx::query(
        "INSERT INTO checkout_sessions (id, cart_id, currency_code, status, created_at, updated_at)
         VALUES (?, ?, 'USD', ?, 0, 0)",
    )
    .bind(id.as_str())
    .bind(CartId::new().as_str())
    .bind(status)
    .execute(db.pool())
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn only_terminal_checkouts_are_abandoned() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CheckoutService::new(db.clone());

    let failed = session(&db, "failed").await;
    let canceled = session(&db, "canceled").await;
    let expired = session(&db, "expired").await;
    let pending = session(&db, "payment_pending").await;
    let placed = session(&db, "order_placed").await;

    let candidates: Vec<String> = [&failed, &canceled, &expired, &pending, &placed]
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();
    let mut abandoned = service.abandoned_ids(&candidates).await.unwrap();
    abandoned.sort();

    let mut expected = vec![
        failed.as_str().to_owned(),
        canceled.as_str().to_owned(),
        expired.as_str().to_owned(),
    ];
    expected.sort();
    assert_eq!(abandoned, expected);
}

#[tokio::test]
async fn unknown_and_empty_candidates_are_ignored() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CheckoutService::new(db);

    assert!(service.abandoned_ids(&[]).await.unwrap().is_empty());
    let unknown = vec![CheckoutId::new().as_str().to_owned()];
    assert!(service.abandoned_ids(&unknown).await.unwrap().is_empty());
}
