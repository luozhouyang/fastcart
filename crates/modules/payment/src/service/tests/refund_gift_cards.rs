//! Refund allocation to gift cards.
//!
//! A checkout paid partly by gift card records the card's share in
//! `payment_gift_card_redemptions`; refunding the payment must restore that
//! money to the card first and send only the remainder to the provider. These
//! tests drive the allocation rule, its idempotency and its concurrency
//! behaviour directly against the payment service.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CheckoutId, OrderId, StoreId};
use tokio::sync::Barrier;

use super::*;
use crate::domain::{CreateGiftCardInput, GiftCard, GiftCardRedemption, Payment, RefundStatus};

/// An order paid by some cards plus a captured provider payment.
struct CardOrder {
    payment: Payment,
    cards: Vec<GiftCard>,
    redemptions: Vec<GiftCardRedemption>,
}

/// Issues `card_amounts`, spends each card on one checkout and links both the
/// redemptions and the captured provider payment to one order.
async fn place_order(
    service: &PaymentService,
    provider: &str,
    card_amounts: &[i64],
    provider_amount: i64,
) -> CardOrder {
    let checkout_id = CheckoutId::new();
    let order_id = OrderId::new();
    let mut cards = Vec::new();
    let mut redemptions = Vec::new();
    for amount in card_amounts {
        let card = service
            .issue_gift_card(CreateGiftCardInput::new("usd", *amount))
            .await
            .unwrap();
        let redemption = service
            .redeem_gift_card_for_checkout(&checkout_id, &card.id, *amount)
            .await
            .unwrap();
        cards.push(card);
        redemptions.push(redemption);
    }
    service
        .assign_gift_card_redemptions_to_order(&checkout_id, &order_id)
        .await
        .unwrap();
    let mut request = input(provider, provider_amount);
    request.order_id = Some(order_id.clone());
    let session = service.create_session(request).await.unwrap();
    let payment = service.capture(session.id.as_str()).await.unwrap();
    CardOrder {
        payment,
        cards,
        redemptions,
    }
}

/// Pins the redemptions to distinct, increasing timestamps.
///
/// The service allocates oldest first and redemptions written in the same
/// second would otherwise fall back to their random identifier order.
async fn pin_redemption_order(db: &StoreDatabase, redemptions: &[GiftCardRedemption]) {
    for (index, redemption) in redemptions.iter().enumerate() {
        sqlx::query("UPDATE payment_gift_card_redemptions SET created_at = ? WHERE id = ?")
            .bind(100 + i64::try_from(index).unwrap())
            .bind(&redemption.id)
            .execute(db.pool())
            .await
            .unwrap();
    }
}

async fn balance(service: &PaymentService, card: &GiftCard) -> i64 {
    service.get_gift_card(&card.id).await.unwrap().balance_minor
}

#[tokio::test]
async fn partial_refunds_credit_the_oldest_redemption_first() {
    let (service, db) = setup().await;
    let provider = Arc::new(TestProvider::new("cards"));
    service.register_provider(provider.clone()).unwrap();
    let order = place_order(&service, "cards", &[400, 300], 1_000).await;
    pin_redemption_order(&db, &order.redemptions).await;

    // 500 is split across the two cards: the oldest redemption gives its whole
    // 400 and the newer one the remaining 100.
    let first = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            500,
            None,
            Some("refund:first".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(first.amount, 500);
    assert_eq!(first.status, RefundStatus::Succeeded);
    assert_eq!(
        first.provider_refund_id, None,
        "a refund fully covered by cards never reaches the provider"
    );
    assert_eq!(provider.refund_count(), 0);
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(balance(&service, &order.cards[1]).await, 100);

    let credits = service
        .gift_card_refund_credits_for_refund(first.id.as_str())
        .await
        .unwrap();
    assert_eq!(credits.len(), 2);
    assert_eq!(credits[0].redemption_id, order.redemptions[0].id);
    assert_eq!(credits[0].amount_minor, 400);
    assert_eq!(credits[1].redemption_id, order.redemptions[1].id);
    assert_eq!(credits[1].amount_minor, 100);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(first.id.as_str())
            .await
            .unwrap(),
        500
    );

    // The second refund can only take what the first left uncredited: 200 from
    // the newer card, and the outstanding 200 falls through to the provider.
    let second = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            400,
            None,
            Some("refund:second".to_owned()),
        )
        .await
        .unwrap();
    assert!(second.provider_refund_id.is_some());
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(provider.refunded_amounts(), vec![200]);
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(balance(&service, &order.cards[1]).await, 300);
    let credits = service
        .gift_card_refund_credits_for_refund(second.id.as_str())
        .await
        .unwrap();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].redemption_id, order.redemptions[1].id);
    assert_eq!(credits[0].amount_minor, 200);
}

#[tokio::test]
async fn a_full_refund_restores_every_card() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("full"));
    service.register_provider(provider.clone()).unwrap();
    let order = place_order(&service, "full", &[300, 200], 1_000).await;

    let refund = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            1_000,
            None,
            Some("refund:full".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(refund.amount, 1_000);
    assert_eq!(refund.status, RefundStatus::Succeeded);
    // Both cards are back to their issued balance, and only the provider part
    // of the payment (1_000 - 500) is refunded through the provider.
    assert_eq!(balance(&service, &order.cards[0]).await, 300);
    assert_eq!(balance(&service, &order.cards[1]).await, 200);
    assert_eq!(provider.refunded_amounts(), vec![500]);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(refund.id.as_str())
            .await
            .unwrap(),
        500
    );
    assert_eq!(
        service
            .get_payment(order.payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
}

#[tokio::test]
async fn a_refund_beyond_the_card_spend_falls_through_to_the_provider() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("remainder"));
    service.register_provider(provider.clone()).unwrap();
    let order = place_order(&service, "remainder", &[400], 1_000).await;

    let refund = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            1_000,
            None,
            Some("refund:remainder".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(refund.amount, 1_000);
    assert!(refund.provider_refund_id.is_some());
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(refund.id.as_str())
            .await
            .unwrap(),
        400
    );
    // Only the part no redemption could absorb reaches the provider.
    assert_eq!(provider.refunded_amounts(), vec![600]);
}

#[tokio::test]
async fn replaying_an_idempotency_key_credits_nothing_new() {
    let (service, _db) = setup().await;
    let provider = Arc::new(TestProvider::new("replay"));
    service.register_provider(provider.clone()).unwrap();
    let order = place_order(&service, "replay", &[400], 1_000).await;

    let first = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            600,
            None,
            Some("return-refund:replay".to_owned()),
        )
        .await
        .unwrap();
    let replay = service
        .refund_with_idempotency(
            order.payment.id.as_str(),
            600,
            None,
            Some("return-refund:replay".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(first.id, replay.id);
    assert_eq!(replay.status, RefundStatus::Succeeded);
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(first.id.as_str())
            .await
            .unwrap(),
        400
    );
    assert_eq!(provider.refund_count(), 1);
    assert_eq!(provider.refunded_amounts(), vec![200]);
    assert_eq!(
        service
            .list_refunds(order.payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn an_ambiguous_failure_keeps_the_credits_and_a_retry_does_not_double_credit() {
    let (service, _db) = setup().await;
    let flaky = Arc::new(TestProvider::failing_refund("flaky"));
    service.register_provider(flaky.clone()).unwrap();
    let order = place_order(&service, "flaky", &[400], 1_000).await;
    let key = "return-refund:flaky";

    let first = service
        .refund_with_idempotency(order.payment.id.as_str(), 600, None, Some(key.to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(first, AppError::ExternalService(_)), "{first}");
    // The provider outcome is unknown, so the refund stays pending and the
    // card money it already restored stays restored.
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    let pending = service
        .list_refunds(order.payment.id.as_str())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, RefundStatus::Pending);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(pending[0].id.as_str())
            .await
            .unwrap(),
        400
    );
    assert_eq!(flaky.refunded_amounts(), vec![200]);

    // A replay takes its own credits back and re-applies them, netting to the
    // same card balance, and asks the provider for the same remainder.
    let second = service
        .refund_with_idempotency(order.payment.id.as_str(), 600, None, Some(key.to_owned()))
        .await
        .unwrap_err();
    assert!(matches!(second, AppError::ExternalService(_)), "{second}");
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(flaky.refunded_amounts(), vec![200, 200]);

    // A working provider on the next attempt lets the pending refund settle.
    service.replace_provider(Arc::new(TestProvider::new("flaky")));
    let settled = service
        .refund_with_idempotency(order.payment.id.as_str(), 600, None, Some(key.to_owned()))
        .await
        .unwrap();
    assert_eq!(settled.status, RefundStatus::Succeeded);
    assert_eq!(balance(&service, &order.cards[0]).await, 400);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(settled.id.as_str())
            .await
            .unwrap(),
        400
    );
    assert_eq!(
        service
            .gift_card_refund_credits_for_refund(settled.id.as_str())
            .await
            .unwrap()
            .len(),
        1,
        "a retry replaces its credit instead of adding another"
    );
}

#[tokio::test]
async fn a_definitive_rejection_takes_the_card_credits_back() {
    let (service, _db) = setup().await;
    service
        .register_provider(Arc::new(TestProvider::invalid_refund("strict")))
        .unwrap();
    let order = place_order(&service, "strict", &[400], 1_000).await;

    let error = service
        .refund(order.payment.id.as_str(), 600, None)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)), "{error}");
    assert_eq!(
        balance(&service, &order.cards[0]).await,
        0,
        "a rejected refund must not leave restored money on the card"
    );
    let refunds = service
        .list_refunds(order.payment.id.as_str())
        .await
        .unwrap();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Failed);
    assert_eq!(
        service
            .gift_card_refunded_for_refund(refunds[0].id.as_str())
            .await
            .unwrap(),
        0
    );
}

/// Two refunds racing for one order must split the redemption, not each credit
/// all of it, and the provider must only see the uncovered remainder.
///
/// The in-memory pool used by [`setup`] is single-connection and would
/// serialize both callers, so this test opens a file-backed pool with real
/// concurrency. The refunds use distinct keys: both are allowed by the
/// over-refund cap (500 + 500 = the captured 1_000), so the only thing that can
/// keep the 600 redemption from being credited twice is the allocation reading
/// the ledger inside each serialized claim.
#[tokio::test]
async fn concurrent_refunds_never_over_credit_one_redemption() {
    let directory = tempfile::tempdir().unwrap();
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        4,
    )
    .await
    .unwrap();
    let service = Arc::new(PaymentService::new(db.clone()));
    let provider = Arc::new(TestProvider::new("racing"));
    service.register_provider(provider.clone()).unwrap();
    let order = place_order(&service, "racing", &[600], 1_000).await;

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for key in ["refund:race-a", "refund:race-b"] {
        let service = service.clone();
        let barrier = barrier.clone();
        let payment_id = order.payment.id.to_string();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .refund_with_idempotency(&payment_id, 500, None, Some(key.to_owned()))
                .await
        }));
    }
    let mut refunds = Vec::new();
    for task in tasks {
        refunds.push(task.await.unwrap().unwrap());
    }
    assert_eq!(refunds.len(), 2);
    for refund in &refunds {
        assert_eq!(refund.status, RefundStatus::Succeeded);
        assert_eq!(refund.amount, 500);
    }

    assert_eq!(
        balance(&service, &order.cards[0]).await,
        600,
        "the card is restored once, never twice"
    );
    let mut credited = 0;
    for refund in &refunds {
        credited += service
            .gift_card_refunded_for_refund(refund.id.as_str())
            .await
            .unwrap();
    }
    assert_eq!(credited, 600, "credits stay within the redemption");
    let provider_refunded: i64 = provider.refunded_amounts().iter().sum();
    assert_eq!(
        provider_refunded, 400,
        "only the part the redemption could not absorb reaches the provider"
    );
    assert_eq!(
        service
            .get_payment(order.payment.id.as_str())
            .await
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
}
