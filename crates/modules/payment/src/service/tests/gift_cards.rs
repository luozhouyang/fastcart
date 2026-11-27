use std::sync::Arc;

use fastcart_kernel::CheckoutId;
use tokio::sync::Barrier;

use super::*;
use crate::domain::{CreateGiftCardInput, GiftCardRedemptionStatus, GiftCardStatus};

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
async fn finds_a_card_by_the_code_a_customer_presents() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 5_000))
        .await
        .unwrap();

    let found = service.find_gift_card_by_code(&card.code).await.unwrap();
    assert_eq!(found.id, card.id);
    assert_eq!(found.currency_code, "USD");
    assert_eq!(found.balance_minor, 5_000);

    let missing = service
        .find_gift_card_by_code("NOT-A-REAL-CODE")
        .await
        .unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)), "{missing}");
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
    // Disabling again is a no-op, including `updated_at`.
    let again = service.disable_gift_card(&card.id).await.unwrap();
    assert_eq!(again.status, GiftCardStatus::Disabled);
    assert_eq!(again.updated_at, disabled.updated_at);
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

#[tokio::test]
async fn checkout_redemption_records_the_ledger_once_and_reverses_idempotently() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 1_000))
        .await
        .unwrap();
    let checkout_id = CheckoutId::new();

    let redemption = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 400)
        .await
        .unwrap();
    assert_eq!(redemption.gift_card_id, card.id);
    assert_eq!(redemption.checkout_id, checkout_id.as_str());
    assert_eq!(redemption.order_id, None);
    assert_eq!(redemption.amount_minor, 400);
    assert_eq!(redemption.status, GiftCardRedemptionStatus::Redeemed);
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        600
    );

    // Replaying the same completion returns the recorded redemption without
    // spending the card again.
    let replayed = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 400)
        .await
        .unwrap();
    assert_eq!(replayed.id, redemption.id);
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        600
    );

    // A different amount under the same key is a conflict, not a silent edit.
    let conflict = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 300)
        .await
        .unwrap_err();
    assert!(matches!(conflict, AppError::Conflict(_)), "{conflict}");

    // The ledger names the order once it exists, and the admin can read it back.
    let order_id = OrderId::new();
    let assigned = service
        .assign_gift_card_redemptions_to_order(&checkout_id, &order_id)
        .await
        .unwrap();
    assert_eq!(assigned, 1);
    let for_card = service
        .gift_card_redemptions_for_card(&card.id)
        .await
        .unwrap();
    assert_eq!(for_card.len(), 1);
    assert_eq!(for_card[0].order_id.as_deref(), Some(order_id.as_str()));

    // Compensation gives the money back exactly once.
    assert_eq!(
        service
            .reverse_gift_card_redemptions(&checkout_id)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        1_000
    );
    assert_eq!(
        service
            .reverse_gift_card_redemptions(&checkout_id)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        1_000
    );

    // A retried completion after the reversal spends the card again and
    // re-arms the same ledger row.
    let rearmed = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 400)
        .await
        .unwrap();
    assert_eq!(rearmed.status, GiftCardRedemptionStatus::Redeemed);
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        600
    );
}

#[tokio::test]
async fn checkout_redemption_rejects_disabled_cards_and_insufficient_balances() {
    let (service, _db) = setup().await;
    let card = service
        .issue_gift_card(CreateGiftCardInput::new("usd", 500))
        .await
        .unwrap();
    let checkout_id = CheckoutId::new();

    let over = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 501)
        .await
        .unwrap_err();
    assert!(matches!(over, AppError::Conflict(_)), "{over}");
    assert_eq!(
        service.get_gift_card(&card.id).await.unwrap().balance_minor,
        500
    );

    service.disable_gift_card(&card.id).await.unwrap();
    let disabled = service
        .redeem_gift_card_for_checkout(&checkout_id, &card.id, 100)
        .await
        .unwrap_err();
    assert!(matches!(disabled, AppError::Conflict(_)), "{disabled}");
    assert!(
        service
            .gift_card_redemptions_for_checkout(&checkout_id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Two checkouts racing for a card that covers only one of them: the guarded
/// decrement must let exactly one win, on a real multi-connection pool.
#[tokio::test]
async fn concurrent_checkout_redemptions_of_one_card_leave_exactly_one_winner() {
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

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        let checkout_id = CheckoutId::new();
        let card_id = card.id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let redeemed = service
                .redeem_gift_card_for_checkout(&checkout_id, &card_id, 700)
                .await;
            (checkout_id, redeemed)
        }));
    }
    let mut winners = Vec::new();
    let mut losers = 0;
    for task in tasks {
        let (checkout_id, result) = task.await.unwrap();
        match result {
            Ok(redemption) => winners.push((checkout_id, redemption)),
            Err(error) => {
                assert!(matches!(error, AppError::Conflict(_)), "{error}");
                losers += 1;
            }
        }
    }
    assert_eq!(winners.len(), 1, "only one checkout may spend the card");
    assert_eq!(losers, 1);

    let stored = service.get_gift_card(&card.id).await.unwrap();
    assert_eq!(stored.balance_minor, 300);
    // The winner's ledger row exists; the loser left none behind.
    let (winner_id, redemption) = &winners[0];
    let recorded = service
        .gift_card_redemptions_for_checkout(winner_id)
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(&recorded[0], redemption);
}
