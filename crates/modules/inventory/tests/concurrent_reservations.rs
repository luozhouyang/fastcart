//! True multi-connection concurrency tests for inventory reservations.
//!
//! The unit tests inside `src/tests.rs` run on `StoreDatabase::in_memory`,
//! whose pool is pinned to a single connection, so their "concurrent" tasks are
//! serialized at the pool boundary and can never interleave. These tests run
//! against a file-backed store with a real multi-connection pool, spawn real
//! tasks on a multi-threaded runtime and assert the invariants the reservation
//! path exists to protect:
//!
//! * total reserved quantity never exceeds stocked quantity,
//! * at most `stocked` reserve calls succeed,
//! * every losing caller fails with an `insufficient stock` conflict,
//! * no reservation row is lost.
//!
//! The last test is a deliberate counterexample: it asserts that a naive
//! check-then-write reservation *does* oversell on this harness, which is what
//! makes the assertions above meaningful rather than vacuously true.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, StoreId, VariantId,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, Reference, ReservationLine,
};
use tokio::sync::Barrier;

async fn setup(max_connections: u32) -> (tempfile::TempDir, StoreDatabase, InventoryService) {
    let directory = tempfile::tempdir().expect("temp dir");
    let db = StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        directory.path().join("store.sqlite"),
        max_connections,
    )
    .await
    .expect("multi-connection store database");
    let service = InventoryService::new(db.clone());
    (directory, db, service)
}

/// Creates one item with `quantity` units stocked at a single location.
async fn stocked_item(
    service: &InventoryService,
    quantity: i64,
) -> (InventoryItemId, InventoryLocationId, VariantId) {
    let location = service
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .expect("create location");
    let variant = VariantId::new();
    let item = service
        .create_item(CreateInventoryItemInput {
            sku: Some(format!("sku-{}", variant.as_str())),
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .expect("create item");
    service
        .adjust_stock(&item.id, &location.id, quantity, "receiving", None)
        .await
        .expect("adjust stock");
    (item.id, location.id, variant)
}

/// Runs `reservers` tasks that all try to reserve `quantity` of `variant` at
/// the same instant, and returns the successful reservation counts together
/// with the failures.
async fn race_reservations(
    service: &Arc<InventoryService>,
    variant: &VariantId,
    quantity: i64,
    reservers: usize,
) -> (Vec<usize>, Vec<AppError>) {
    let barrier = Arc::new(Barrier::new(reservers));
    let mut tasks = Vec::new();
    for _ in 0..reservers {
        let service = service.clone();
        let barrier = barrier.clone();
        let variant = variant.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service
                .reserve(
                    vec![ReservationLine::new(variant, quantity)],
                    Reference::new(CheckoutId::new()),
                    60,
                )
                .await
        }));
    }

    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for task in tasks {
        match task.await.expect("reserve task panicked") {
            Ok(reservations) => successes.push(reservations.len()),
            Err(error) => failures.push(error),
        }
    }
    (successes, failures)
}

async fn reserved_at_level(db: &StoreDatabase, item: &InventoryItemId) -> i64 {
    sqlx::query_scalar("SELECT reserved_quantity FROM inventory_levels WHERE inventory_item_id = ?")
        .bind(item.as_str())
        .fetch_one(db.pool())
        .await
        .expect("read inventory level")
}

async fn reserved_rows(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity), 0) FROM inventory_reservations WHERE status = 'reserved'",
    )
    .fetch_one(db.pool())
    .await
    .expect("sum reservations")
}

fn assert_insufficient_stock(failures: &[AppError]) {
    for failure in failures {
        assert!(
            matches!(failure, AppError::Conflict(message) if message.contains("insufficient stock")),
            "a losing reservation must fail with an insufficient stock conflict, got: {failure}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reservations_never_oversell_the_last_unit() {
    let (_directory, db, service) = setup(8).await;
    let (item, location, variant) = stocked_item(&service, 1).await;
    let service = Arc::new(service);

    let (successes, failures) = race_reservations(&service, &variant, 1, 8).await;

    assert_eq!(
        successes.len(),
        1,
        "exactly one of eight concurrent reservations may win the last unit"
    );
    assert_eq!(failures.len(), 7);
    assert_insufficient_stock(&failures);
    assert!(reserved_rows(&db).await <= 1, "reservations exceed stock");
    assert_eq!(reserved_at_level(&db, &item).await, 1);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reservations_never_exceed_the_stocked_quantity() {
    let (_directory, db, service) = setup(8).await;
    let (item, location, variant) = stocked_item(&service, 5).await;
    let service = Arc::new(service);

    let (successes, failures) = race_reservations(&service, &variant, 1, 24).await;

    assert_eq!(
        successes.len(),
        5,
        "twenty-four concurrent reservations must spend exactly the five units in stock"
    );
    assert_eq!(failures.len(), 19);
    assert_insufficient_stock(&failures);
    assert_eq!(
        reserved_rows(&db).await,
        5,
        "every winning reservation must be persisted"
    );
    assert_eq!(reserved_at_level(&db, &item).await, 5);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_multi_unit_reservations_do_not_split_the_last_units_twice() {
    let (_directory, db, service) = setup(4).await;
    let (item, location, variant) = stocked_item(&service, 3).await;
    let service = Arc::new(service);

    // Four callers each need two units; only one of them can be satisfied.
    let (successes, failures) = race_reservations(&service, &variant, 2, 4).await;

    assert_eq!(
        successes,
        vec![1],
        "exactly one two-unit reservation may win"
    );
    assert_eq!(failures.len(), 3);
    assert_insufficient_stock(&failures);
    assert_eq!(reserved_rows(&db).await, 2);
    assert_eq!(reserved_at_level(&db, &item).await, 2);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_naive_check_then_write_reserve_oversells_on_this_harness() {
    // Counterexample, deliberately asserting the bug. Two connections read the
    // availability, then both write an absolute reserved quantity computed from
    // that stale read. This is the shape of the bug the tests above must catch,
    // and it is reproduced here to prove that the harness interleaves the two
    // callers rather than serializing them.
    let (_directory, db, service) = setup(2).await;
    let (item, location, _variant) = stocked_item(&service, 1).await;

    let barrier = Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for index in 0..2 {
        let db = db.clone();
        let barrier = barrier.clone();
        let item = item.clone();
        let location = location.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = db.pool().acquire().await.unwrap();
            let (stocked, reserved): (i64, i64) = sqlx::query_as(
                "SELECT stocked_quantity, reserved_quantity FROM inventory_levels \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(item.as_str())
            .bind(location.as_str())
            .fetch_one(&mut *connection)
            .await
            .unwrap();
            let available = stocked - reserved;
            barrier.wait().await;
            if available <= 0 {
                return available;
            }
            // Naive act: the write is based on the value read before the barrier.
            sqlx::query(
                "UPDATE inventory_levels SET reserved_quantity = ? \
                 WHERE inventory_item_id = ? AND location_id = ?",
            )
            .bind(reserved + 1)
            .bind(item.as_str())
            .bind(location.as_str())
            .execute(&mut *connection)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO inventory_reservations \
                 (id, inventory_item_id, location_id, quantity, status, created_at, updated_at) \
                 VALUES (?, ?, ?, 1, 'reserved', 0, 0)",
            )
            .bind(format!("res_{index}_{}", item.as_str()))
            .bind(item.as_str())
            .bind(location.as_str())
            .execute(&mut *connection)
            .await
            .unwrap();
            available
        }));
    }

    let mut observed = Vec::new();
    for task in tasks {
        observed.push(task.await.unwrap());
    }
    observed.sort_unstable();

    assert_eq!(
        observed,
        vec![1, 1],
        "both callers must observe the last unit as available"
    );
    assert_eq!(
        reserved_rows(&db).await,
        2,
        "two reservations were recorded against one stocked unit"
    );
    assert_eq!(
        reserved_at_level(&db, &item).await,
        1,
        "the level lost one reservation: the harness reproduces the oversell"
    );
}
