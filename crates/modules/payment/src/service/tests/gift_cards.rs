use std::sync::Arc;

use tokio::sync::Barrier;

use super::*;
use crate::domain::{CreateGiftCardInput, GiftCardStatus};

#[tokio::test]
async fn issues_generates_and_lists_gift_cards() {
    let (service, _db) = setup().await;
    let issued = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 2_500))
        .await
        .unwrap();
    assert_eq!(issued.currency_code, "USD");
    assert_eq!(issued.initial_amount_minor, 2_500);
    assert_eq!(issued.balance_minor, 2_500);
    assert_eq!(issued.status, GiftCardStatus::Active);
    assert_eq!(issued.code.chars().count(), 16);
    assert!(
        issued
            .code
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit()),
        "generated code must be unambiguous uppercase alphanumerics: {}",
        issued.code
    );
    assert!(issued.id.starts_with("gift_"));

    let explicit = service
        .issue_gift_card(CreateGiftCardInput {
            currency_code: "eur".to_owned(),
            amount_minor: 1_000,
            code: Some("SUMMER-2024".to_owned()),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    assert_eq!(explicit.code, "SUMMER-2024");

    let page = service
        .list_gift_cards(Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 2);

    let page = service
        .list_gift_cards(Pagination::new(Some(1), Some(1)).unwrap())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let fetched = service.get_gift_card(&issued.id).await.unwrap();
    assert_eq!(fetched.code, issued.code);
}

#[tokio::test]
async fn issuing_validates_amount_currency_and_code() {
    let (service, _db) = setup().await;
    let zero = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 0))
        .await
        .unwrap_err();
    assert!(matches!(zero, AppError::Validation(_)));
    let negative = service
        .issue_gift_card(CreateGiftCardInput::new("usd", -100))
        .await
        .unwrap_err();
    assert!(matches!(negative, AppError::Validation(_)));
    let currency = service
        .issue_gift_card(CreateGiftCardInput::new("not-a-currency", 100))
        .await
        .unwrap_err();
    assert!(matches!(currency, AppError::Validation(_)));
    let empty_code = service
        .issue_gift_card(CreateGiftCardInput {
            currency_code: "usd".to_owned(),
            amount_minor: 100,
            code: Some("   ".to_owned()),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap_err();
    assert!(matches!(empty_code, AppError::Validation(_)));

    service
        .issue_gift_card(CreateGiftCardInput {
            currency_code: "usd".to_owned(),
            amount_minor: 100,
            code: Some("DUPLICATE".to_owned()),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let duplicate = service
        .issue_gift_card(CreateGiftCardInput {
            currency_code: "usd".to_owned(),
            amount_minor: 500,
            code: Some("DUPLICATE".to_owned()),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap_err();
    assert!(matches!(duplicate, AppError::Conflict(_)), "{duplicate}");

    let missing = service.get_gift_card("gift_missing").await.unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)));
}

#[tokio::test]
async fn redeems_against_the_remaining_balance() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 1_000))
        .await
        .unwrap();

    let balance = service.redeem_gift_card(&card.code, 400).await.unwrap();
    assert_eq!(balance, 600);

    // Redeeming the remainder drains the card but does not disable it; whether
    // an exhausted card stays redeemable is the caller's business.
    let balance = service.redeem_gift_card(&card.code, 600).await.unwrap();
    assert_eq!(balance, 0);

    // A code was issued in another case and must match exactly.
    let mismatch = service.redeem_gift_card(&card.code.to_lowercase(), 1).await;
    assert!(matches!(mismatch, Err(AppError::NotFound(_))));
}

#[tokio::test]
async fn redemption_rejects_insufficient_balance_zero_and_unknown_codes() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 1_000))
        .await
        .unwrap();

    let over = service
        .redeem_gift_card(&card.code, 1_001)
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)), "{over}");
    let zero = service.redeem_gift_card(&card.code, 0).await.unwrap_err();
    assert!(matches!(zero, AppError::Validation(_)));
    let negative = service.redeem_gift_card(&card.code, -1).await.unwrap_err();
    assert!(matches!(negative, AppError::Validation(_)));
    let unknown = service
        .redeem_gift_card("NO-SUCH-CODE", 1)
        .await
        .unwrap_err();
    assert!(matches!(unknown, AppError::NotFound(_)));

    // Rejected redemptions never moved the balance.
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        1_000
    );
}

#[tokio::test]
async fn disabled_cards_cannot_be_redeemed_until_enabled() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 1_000))
        .await
        .unwrap();

    let disabled = service.disable_gift_card(&card.id).await.unwrap();
    assert_eq!(disabled.status, GiftCardStatus::Disabled);
    // Disabling again is a no-op.
    assert_eq!(
        service.disable_gift_card(&card.id).await.unwrap().status,
        GiftCardStatus::Disabled
    );
    let rejected = service.redeem_gift_card(&card.code, 100).await.unwrap_err();
    assert!(matches!(rejected, AppError::Conflict(_)), "{rejected}");
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        1_000
    );

    let enabled = service.enable_gift_card(&card.id).await.unwrap();
    assert_eq!(enabled.status, GiftCardStatus::Active);
    let balance = service.redeem_gift_card(&card.code, 100).await.unwrap();
    assert_eq!(balance, 900);
}

/// True multi-connection race at the redemption boundary.
///
/// `redeem_gift_card` runs a conditional `UPDATE ... WHERE balance_minor >= ?`
/// and checks `rows_affected`, so when two callers race for a balance that only
/// covers one of them, the loser must fail instead of driving the balance
/// negative. The in-memory pool used by [`setup`] is single-connection and
/// would serialize both callers, so this test opens a file-backed pool with
/// real concurrency.
#[tokio::test]
async fn concurrent_redemptions_at_the_boundary_never_oversell() {
    let directory = tempfile::tempdir().unwrap();
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        4,
    )
    .await
    .unwrap();
    let service = Arc::new(PaymentService::new(db.clone()));
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 1_000))
        .await
        .unwrap();
    assert_eq!(card.balance_minor, 1_000);

    // Each caller redeems 700 of a 1_000 balance: exactly one can win.
    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        let code = card.code.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service.redeem_gift_card(&code, 700).await
        }));
    }
    let mut successes = 0;
    let mut failures = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(balance) => {
                successes += 1;
                assert_eq!(balance, 300);
            }
            Err(error) => {
                assert!(matches!(error, AppError::Conflict(_)), "{error}");
                failures += 1;
            }
        }
    }
    assert_eq!(successes, 1, "only one redemption may win the race");
    assert_eq!(failures, 1, "the losing redemption must fail");

    let stored = service.get_gift_card(&card.id).await.unwrap();
    assert_eq!(stored.balance_minor, 300, "the balance never oversells");
    let source: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gift_cards")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(source, 1);
}
