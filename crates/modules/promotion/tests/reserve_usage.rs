//! Reservation lifecycle for promotion usage.
//!
//! `reserve_usage` claims a usage atomically before money moves and keys the
//! claim to the checkout (`checkout:<id>`); `reassign_usage` moves the claim
//! onto the order that was actually placed; `release_usage` gives a claim back
//! when the checkout never becomes an order.
//!
//! These tests run against a file-backed store with a real multi-connection
//! pool, because a check-then-write reservation only fails under an actual
//! race. `concurrent_usage.rs` documents the counterexample that proves this
//! harness interleaves.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CurrencyCode, StoreId};
use fastcart_module_promotion::{
    CampaignBudgetType, CreateCampaignInput, CreateCouponInput, CreatePromotionInput,
    PromotionService, PromotionType,
};
use tokio::sync::Barrier;

async fn setup(max_connections: u32) -> (tempfile::TempDir, StoreDatabase, PromotionService) {
    let directory = tempfile::tempdir().expect("temp dir");
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        max_connections,
    )
    .await
    .expect("multi-connection store database");
    let service = PromotionService::new(db.clone());
    (directory, db, service)
}

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn assert_conflict(error: &AppError, expected: &str) {
    assert!(
        matches!(error, AppError::Conflict(message) if message.contains(expected)),
        "expected a conflict containing {expected:?}, got: {error}"
    );
}

async fn usage_rows(db: &StoreDatabase, promotion_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage WHERE promotion_id = ?")
        .bind(promotion_id)
        .fetch_one(db.pool())
        .await
        .expect("count usage rows")
}

async fn rows_for_key(db: &StoreDatabase, key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage WHERE order_id = ?")
        .bind(key)
        .fetch_one(db.pool())
        .await
        .expect("count rows for key")
}

async fn stored_order_id(db: &StoreDatabase, promotion_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT order_id FROM promotion_usage WHERE promotion_id = ?")
        .bind(promotion_id)
        .fetch_one(db.pool())
        .await
        .expect("read the recorded order id")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reservations_never_exceed_the_promotion_limit() {
    let (_directory, db, service) = setup(8).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Limited", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(1),
        )
        .await
        .unwrap();
    let promotion_id = promotion.id.clone();
    let service = Arc::new(service);

    // Eight checkouts race for a promotion that may be used once.
    let barrier = Arc::new(Barrier::new(8));
    let mut tasks = Vec::new();
    for index in 0..8 {
        let service = service.clone();
        let barrier = barrier.clone();
        let promotion_id = promotion_id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .reserve_usage(
                    &promotion_id,
                    None,
                    None,
                    &format!("checkout_{index}"),
                    0,
                    None,
                )
                .await
        }));
    }

    let mut winners = Vec::new();
    let mut failures = Vec::new();
    for task in tasks {
        match task.await.expect("reserve_usage task panicked") {
            Ok(usage) => winners.push(usage),
            Err(error) => failures.push(error),
        }
    }

    assert_eq!(
        winners.len(),
        1,
        "exactly one of eight concurrent checkouts may claim a limit of one"
    );
    assert_eq!(failures.len(), 7);
    for failure in &failures {
        assert_conflict(failure, "promotion usage limit reached");
    }
    assert_eq!(service.usage_count(&promotion_id).await.unwrap(), 1);
    assert_eq!(usage_rows(&db, &promotion_id).await, 1);

    // The winner's claim is keyed to its checkout, not to an order yet.
    let claimed = winners[0]
        .order_id
        .clone()
        .expect("a claim carries its key");
    assert!(
        claimed.starts_with("checkout:checkout_"),
        "unexpected claim key: {claimed}"
    );
    assert_eq!(rows_for_key(&db, &claimed).await, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repeated_reservation_for_the_same_checkout_counts_once() {
    let (_directory, db, service) = setup(4).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Idempotent", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(1),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Spend budget".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(usd()),
            budget_limit: Some(1_000),
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();
    let service = Arc::new(service);

    // A retried checkout completion re-runs the same reservation.
    let barrier = Arc::new(Barrier::new(6));
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let service = service.clone();
        let barrier = barrier.clone();
        let promotion_id = promotion.id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .reserve_usage(
                    &promotion_id,
                    None,
                    None,
                    "checkout_replayed",
                    400,
                    Some("USD"),
                )
                .await
        }));
    }

    let mut usages = Vec::new();
    for task in tasks {
        usages.push(
            task.await
                .expect("reserve_usage task panicked")
                .expect("a replay must return the winning claim instead of failing"),
        );
    }

    let winner = &usages[0].id;
    assert!(
        usages.iter().all(|usage| &usage.id == winner),
        "every replay must observe the single winning claim"
    );
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(usage_rows(&db, &promotion.id).await, 1);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        400,
        "a replay must not charge the campaign budget twice"
    );
}

#[tokio::test]
async fn releasing_a_claim_restores_every_counter() {
    let (_directory, db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Releasable", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(5),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("RELEASE", &promotion.id).with_usage_limit(5))
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Spend budget".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(usd()),
            budget_limit: Some(1_000),
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    let key = "checkout:checkout_1";
    service
        .reserve_usage(
            &promotion.id,
            Some(&coupon.id),
            Some("cust_1"),
            "checkout_1",
            400,
            Some("USD"),
        )
        .await
        .unwrap();
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 1);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        400
    );

    assert_eq!(service.release_usage(key).await.unwrap(), 1);
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 0);
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 0);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        0,
        "the released spend must be given back to the campaign"
    );
    assert_eq!(usage_rows(&db, &promotion.id).await, 0);

    // The promotion is claimable again after the release.
    service
        .reserve_usage(
            &promotion.id,
            Some(&coupon.id),
            Some("cust_1"),
            "checkout_1",
            400,
            Some("USD"),
        )
        .await
        .unwrap();
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        400
    );

    // A second release is a no-op and never drives a counter negative.
    assert_eq!(service.release_usage(key).await.unwrap(), 1);
    assert_eq!(service.release_usage(key).await.unwrap(), 0);
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 0);
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 0);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        0
    );
}

#[tokio::test]
async fn releasing_a_usage_campaign_budget_returns_one_unit() {
    let (_directory, _db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Counted", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Usage budget".to_owned(),
            budget_type: CampaignBudgetType::Usage,
            currency_code: None,
            budget_limit: Some(3),
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    service
        .reserve_usage(&promotion.id, None, None, "checkout_1", 400, Some("USD"))
        .await
        .unwrap();
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        1,
        "a usage campaign consumes one unit regardless of the discount"
    );

    service.release_usage("checkout:checkout_1").await.unwrap();
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        0,
        "releasing a usage campaign returns exactly one unit"
    );
}

#[tokio::test]
async fn releasing_a_checkout_releases_every_promotion_it_claimed() {
    let (_directory, db, service) = setup(2).await;
    let first = service
        .create_promotion(
            CreatePromotionInput::new("First", PromotionType::FixedAmount, 100).active(),
        )
        .await
        .unwrap();
    let second = service
        .create_promotion(
            CreatePromotionInput::new("Second", PromotionType::FixedAmount, 200).active(),
        )
        .await
        .unwrap();

    for promotion_id in [&first.id, &second.id] {
        service
            .reserve_usage(
                promotion_id,
                None,
                Some("cust_1"),
                "checkout_1",
                100,
                Some("USD"),
            )
            .await
            .unwrap();
    }
    assert_eq!(usage_rows(&db, &first.id).await, 1);
    assert_eq!(usage_rows(&db, &second.id).await, 1);

    assert_eq!(
        service.release_usage("checkout:checkout_1").await.unwrap(),
        2
    );
    assert_eq!(service.usage_count(&first.id).await.unwrap(), 0);
    assert_eq!(service.usage_count(&second.id).await.unwrap(), 0);
}

#[tokio::test]
async fn per_customer_limits_are_enforced() {
    let (_directory, db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Once per customer", PromotionType::FixedAmount, 500)
                .active()
                .with_per_customer_limit(1),
        )
        .await
        .unwrap();
    assert_eq!(promotion.per_customer_limit, Some(1));

    service
        .reserve_usage(&promotion.id, None, Some("cust_a"), "checkout_1", 0, None)
        .await
        .unwrap();
    assert_conflict(
        &service
            .reserve_usage(&promotion.id, None, Some("cust_a"), "checkout_2", 0, None)
            .await
            .unwrap_err(),
        "per-customer usage limit reached",
    );

    // Another customer is unaffected, and a replay of the first checkout is
    // still idempotent.
    service
        .reserve_usage(&promotion.id, None, Some("cust_b"), "checkout_3", 0, None)
        .await
        .unwrap();
    service
        .reserve_usage(&promotion.id, None, Some("cust_a"), "checkout_1", 0, None)
        .await
        .unwrap();
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 2);
    assert_eq!(usage_rows(&db, &promotion.id).await, 2);

    // A release gives the customer their slot back.
    service.release_usage("checkout:checkout_1").await.unwrap();
    service
        .reserve_usage(&promotion.id, None, Some("cust_a"), "checkout_4", 0, None)
        .await
        .unwrap();

    // Without a per-customer limit the same customer may claim repeatedly.
    let unlimited = service
        .create_promotion(
            CreatePromotionInput::new("Unlimited", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    for checkout_id in ["checkout_5", "checkout_6"] {
        service
            .reserve_usage(&unlimited.id, None, Some("cust_a"), checkout_id, 0, None)
            .await
            .unwrap();
    }
    assert_eq!(service.usage_count(&unlimited.id).await.unwrap(), 2);
}

#[tokio::test]
async fn per_customer_limits_must_be_positive() {
    let service = PromotionService::new(StoreDatabase::in_memory(StoreId::new()).await.unwrap());

    for limit in [0, -1] {
        let error = service
            .create_promotion(
                CreatePromotionInput::new("Invalid", PromotionType::FixedAmount, 500)
                    .with_per_customer_limit(limit),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::Validation(message) if message.contains("per_customer_limit")),
            "a per_customer_limit of {limit} must be rejected, got: {error}"
        );
    }
}

#[tokio::test]
async fn reassigning_a_claim_moves_it_onto_the_order_without_recounting() {
    let (_directory, db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Reassignable", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let campaign = service
        .create_campaign(CreateCampaignInput {
            name: "Spend budget".to_owned(),
            budget_type: CampaignBudgetType::Spend,
            currency_code: Some(usd()),
            budget_limit: Some(1_000),
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .link_promotion(&campaign.id, &promotion.id)
        .await
        .unwrap();

    let key = "checkout:checkout_1";
    service
        .reserve_usage(&promotion.id, None, None, "checkout_1", 400, Some("USD"))
        .await
        .unwrap();

    assert_eq!(service.reassign_usage(key, "order_1").await.unwrap(), 1);
    assert_eq!(
        stored_order_id(&db, &promotion.id).await.as_deref(),
        Some("order_1")
    );
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
    assert_eq!(
        service
            .get_campaign(&campaign.id)
            .await
            .unwrap()
            .budget_used,
        400,
        "reassigning a claim must not touch any counter"
    );

    // The placeholder key is gone, so a replay is a no-op.
    assert_eq!(service.reassign_usage(key, "order_1").await.unwrap(), 0);
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
}

#[tokio::test]
async fn reassigning_onto_an_order_that_already_claimed_drops_the_duplicate() {
    let (_directory, db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Converging", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();

    for checkout_id in ["checkout_1", "checkout_2"] {
        service
            .reserve_usage(&promotion.id, None, None, checkout_id, 0, None)
            .await
            .unwrap();
    }
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 2);

    assert_eq!(
        service
            .reassign_usage("checkout:checkout_1", "order_1")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .reassign_usage("checkout:checkout_2", "order_1")
            .await
            .unwrap(),
        1,
        "the duplicate claim is dropped, not rejected"
    );
    assert_eq!(
        usage_rows(&db, &promotion.id).await,
        1,
        "the surviving row is the one attached to the order"
    );
    assert_eq!(
        stored_order_id(&db, &promotion.id).await.as_deref(),
        Some("order_1")
    );
    assert_eq!(
        service.usage_count(&promotion.id).await.unwrap(),
        2,
        "dropping a duplicate claim must not change any counter"
    );
}

#[tokio::test]
async fn releasing_a_key_that_was_never_claimed_is_a_no_op() {
    let (_directory, _db, service) = setup(2).await;
    assert_eq!(service.release_usage("checkout:missing").await.unwrap(), 0);
    assert_eq!(
        service
            .reassign_usage("checkout:missing", "order_1")
            .await
            .unwrap(),
        0
    );
}

/// The per-customer limit is part of the promotion, so an update has to be able
/// to set it and clear it again. Without this it could only ever be chosen when
/// the promotion was created.
#[tokio::test]
async fn updating_a_promotion_changes_its_per_customer_limit() {
    let (_directory, _db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Limited", PromotionType::Percentage, 1_000).active(),
        )
        .await
        .unwrap();
    assert_eq!(promotion.per_customer_limit, None);

    let updated = service
        .update_promotion(
            &promotion.id,
            fastcart_module_promotion::UpdatePromotionInput {
                per_customer_limit: Some(Some(2)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.per_customer_limit, Some(2));
    assert_eq!(
        service
            .get_promotion(&promotion.id)
            .await
            .unwrap()
            .per_customer_limit,
        Some(2),
        "the limit must be persisted, not just echoed back"
    );

    let cleared = service
        .update_promotion(
            &promotion.id,
            fastcart_module_promotion::UpdatePromotionInput {
                per_customer_limit: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.per_customer_limit, None);
}
