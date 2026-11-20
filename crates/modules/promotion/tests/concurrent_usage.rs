//! True multi-connection concurrency tests for promotion usage accounting.
//!
//! The `usage_limits_are_enforced` unit test inside `src/service.rs` calls
//! `record_usage` sequentially on a single-connection in-memory database, so it
//! only proves that a *second* call is rejected after the first one committed.
//! It cannot exercise the interleaving that matters: two callers reading the
//! same `usage_count` and both deciding they are within the limit.
//!
//! These tests run against a file-backed store with a real multi-connection
//! pool and race the callers for real. They assert:
//!
//! * the recorded usage count never exceeds the configured limit,
//! * losing callers fail with a usage-limit conflict,
//! * a duplicated order identifier records exactly one usage row,
//! * the promotion, coupon and `promotion_usage` counters agree.
//!
//! The last test is a deliberate counterexample: it asserts that a naive
//! read-check-then-write recording *does* exceed the limit on this harness.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_promotion::{
    CreateCouponInput, CreatePromotionInput, PromotionService, PromotionType, PromotionUsage,
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

/// Races `order_ids.len()` concurrent `record_usage` calls for the same
/// promotion and returns the recorded usages and the failures.
async fn race_usage(
    service: &Arc<PromotionService>,
    promotion_id: &str,
    coupon_id: Option<&str>,
    order_ids: Vec<String>,
) -> (Vec<PromotionUsage>, Vec<AppError>) {
    let barrier = Arc::new(Barrier::new(order_ids.len()));
    let mut tasks = Vec::new();
    for order_id in order_ids {
        let service = service.clone();
        let barrier = barrier.clone();
        let promotion_id = promotion_id.to_owned();
        let coupon_id = coupon_id.map(str::to_owned);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .record_usage(
                    &promotion_id,
                    coupon_id.as_deref(),
                    None,
                    Some(&order_id),
                    0,
                    None,
                )
                .await
        }));
    }

    let mut usages = Vec::new();
    let mut failures = Vec::new();
    for task in tasks {
        match task.await.expect("record_usage task panicked") {
            Ok(usage) => usages.push(usage),
            Err(error) => failures.push(error),
        }
    }
    (usages, failures)
}

async fn usage_rows(db: &StoreDatabase, promotion_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage WHERE promotion_id = ?")
        .bind(promotion_id)
        .fetch_one(db.pool())
        .await
        .expect("count usage rows")
}

fn assert_conflict(failures: &[AppError], expected: &str) {
    for failure in failures {
        assert!(
            matches!(failure, AppError::Conflict(message) if message.contains(expected)),
            "expected a conflict containing {expected:?}, got: {failure}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_usage_never_exceeds_the_promotion_limit() {
    let (_directory, db, service) = setup(8).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Limited", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(3),
        )
        .await
        .unwrap();
    let service = Arc::new(service);

    let orders: Vec<String> = (0..16).map(|index| format!("order_{index}")).collect();
    let (usages, failures) = race_usage(&service, &promotion.id, None, orders).await;

    assert_eq!(
        usages.len(),
        3,
        "exactly three of sixteen concurrent callers may consume a limit of three"
    );
    assert_eq!(failures.len(), 13);
    assert_conflict(&failures, "promotion usage limit reached");
    assert_eq!(
        service.usage_count(&promotion.id).await.unwrap(),
        3,
        "the recorded usage count must match the winning callers"
    );
    assert_eq!(
        usage_rows(&db, &promotion.id).await,
        3,
        "one usage row per winning caller"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_usage_never_exceeds_the_coupon_limit() {
    let (_directory, db, service) = setup(8).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Coupon limited", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let coupon = service
        .create_coupon(CreateCouponInput::new("RACE", &promotion.id).with_usage_limit(2))
        .await
        .unwrap();
    assert!(coupon.usage_limit == Some(2));
    let service = Arc::new(service);

    let orders: Vec<String> = (0..12).map(|index| format!("order_{index}")).collect();
    let (usages, failures) = race_usage(&service, &promotion.id, Some(&coupon.id), orders).await;

    assert_eq!(
        usages.len(),
        2,
        "exactly two of twelve concurrent callers may consume a coupon limit of two"
    );
    assert_eq!(failures.len(), 10);
    assert_conflict(&failures, "coupon usage limit reached");
    assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 2);
    assert_eq!(
        service.usage_count(&promotion.id).await.unwrap(),
        2,
        "a rejected coupon redemption must not increment the promotion counter"
    );
    assert_eq!(usage_rows(&db, &promotion.id).await, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_duplicated_order_records_exactly_one_usage_row() {
    let (_directory, db, service) = setup(8).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Idempotent", PromotionType::FixedAmount, 500).active(),
        )
        .await
        .unwrap();
    let service = Arc::new(service);

    // Eight callers replay the same order, as a retried checkout completion
    // would.
    let orders: Vec<String> = std::iter::repeat_n("order_replayed".to_owned(), 8).collect();
    let (usages, failures) = race_usage(&service, &promotion.id, None, orders).await;

    assert!(
        failures.is_empty(),
        "a replay must return the winning usage instead of failing: {failures:?}"
    );
    assert_eq!(usages.len(), 8);
    let winner = &usages[0].id;
    assert!(
        usages.iter().all(|usage| &usage.id == winner),
        "every replay must observe the single winning usage row"
    );
    assert_eq!(usage_rows(&db, &promotion.id).await, 1);
    assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_naive_check_then_write_recording_violates_the_limit_on_this_harness() {
    // Counterexample, deliberately asserting the bug: two connections read
    // `usage_count` (both see 0 against a limit of 1), both insert a usage row
    // and both write back `read + 1`. The limit is then exceeded while the
    // counter reports it was respected. This proves the races above interleave
    // for real.
    let (_directory, db, service) = setup(2).await;
    let promotion = service
        .create_promotion(
            CreatePromotionInput::new("Naive", PromotionType::FixedAmount, 500)
                .active()
                .with_usage_limit(1),
        )
        .await
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for index in 0..2 {
        let db = db.clone();
        let barrier = barrier.clone();
        let promotion_id = promotion.id.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = db.pool().acquire().await.unwrap();
            let (limit, count): (Option<i64>, i64) =
                sqlx::query_as("SELECT usage_limit, usage_count FROM promotions WHERE id = ?")
                    .bind(&promotion_id)
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap();
            barrier.wait().await;
            // Naive check: both callers believe they are within the limit.
            if limit.is_some_and(|limit| count >= limit) {
                return None;
            }
            sqlx::query(
                "INSERT INTO promotion_usage \
                 (id, promotion_id, coupon_id, customer_id, order_id, created_at) \
                 VALUES (?, ?, NULL, NULL, ?, 0)",
            )
            .bind(format!("usage_{index}"))
            .bind(&promotion_id)
            .bind(format!("order_{index}"))
            .execute(&mut *connection)
            .await
            .unwrap();
            // Naive write: the counter is set from the value read before the
            // barrier, so one increment is lost.
            sqlx::query("UPDATE promotions SET usage_count = ? WHERE id = ?")
                .bind(count + 1)
                .bind(&promotion_id)
                .execute(&mut *connection)
                .await
                .unwrap();
            Some(count)
        }));
    }

    let mut observed = Vec::new();
    for task in tasks {
        observed.push(task.await.unwrap());
    }

    assert_eq!(
        observed,
        vec![Some(0), Some(0)],
        "both callers must observe usage_count 0 against a limit of 1"
    );
    assert_eq!(
        usage_rows(&db, &promotion.id).await,
        2,
        "two usages were recorded"
    );
    assert_eq!(
        service.usage_count(&promotion.id).await.unwrap(),
        1,
        "the counter lost one increment: the harness reproduces the limit violation"
    );
}
