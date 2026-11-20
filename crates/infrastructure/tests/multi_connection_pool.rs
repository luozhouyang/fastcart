//! The multi-connection test harness that concurrency tests depend on.
//!
//! `StoreDatabase::in_memory` pins its pool to a single connection, so
//! "concurrent" tasks sharing that handle are serialized at the pool boundary
//! and no interleaving is ever exercised. These tests pin down the file-backed
//! multi-connection constructor and prove that it can actually reproduce the
//! interleavings (in particular lost updates) that the module concurrency
//! tests are meant to catch.

use std::time::Duration;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use tokio::sync::Barrier;

async fn store(dir: &tempfile::TempDir, max_connections: u32) -> StoreDatabase {
    StoreDatabase::file_backed_multi_connection(
        StoreId::new(),
        dir.path().join("store.sqlite"),
        max_connections,
    )
    .await
    .expect("open a multi-connection store database")
}

#[tokio::test]
async fn migrations_are_applied_to_the_file_backed_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = store(&dir, 4).await;

    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
           AND name IN ('settings', 'inventory_levels', 'promotion_usage', 'payment_sessions')",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(tables, 4, "the store migrator does not run in parallel");
}

#[tokio::test]
async fn the_pool_hands_out_more_than_one_connection() {
    let dir = tempfile::tempdir().unwrap();
    let db = store(&dir, 4).await;

    let first = db.pool().acquire().await.unwrap();
    let second = db.pool().acquire().await.unwrap();
    assert!(
        db.pool().size() >= 2,
        "expected two live connections, got {}",
        db.pool().size()
    );
    drop(first);
    drop(second);
}

#[tokio::test]
async fn in_memory_pools_are_pinned_to_a_single_connection() {
    // This is the defect the harness exists for: on an in-memory pool a second
    // task can never overlap with the first, because it cannot even acquire a
    // connection until the first one is released.
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let held = db.pool().acquire().await.unwrap();

    let second = tokio::time::timeout(Duration::from_millis(200), db.pool().acquire()).await;
    assert!(
        second.is_err(),
        "a single-connection in-memory pool must not hand out a second handle"
    );
    assert_eq!(db.pool().size(), 1);
    drop(held);
}

#[tokio::test]
async fn a_naive_read_modify_write_loses_an_update_on_this_harness() {
    // Counterexample, deliberately asserting the *bug*: two writers that read a
    // value and write `read + 1` back are interleaved here and one increment is
    // lost. Without this, the module concurrency tests would be vacuous; with
    // it, an unguarded implementation is known to fail them.
    let dir = tempfile::tempdir().unwrap();
    let db = store(&dir, 2).await;
    sqlx::query("CREATE TABLE probe (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO probe (id, value) VALUES (1, 0)")
        .execute(db.pool())
        .await
        .unwrap();

    let barrier = std::sync::Arc::new(Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let db = db.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            let mut connection = db.pool().acquire().await.unwrap();
            let value: i64 = sqlx::query_scalar("SELECT value FROM probe WHERE id = 1")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            // Both writers release their stale read before either writes back.
            barrier.wait().await;
            sqlx::query("UPDATE probe SET value = ? WHERE id = 1")
                .bind(value + 1)
                .execute(&mut *connection)
                .await
                .unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    let value: i64 = sqlx::query_scalar("SELECT value FROM probe WHERE id = 1")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        value, 1,
        "two concurrent read-modify-write writers must be able to lose an update \
         on this harness; got {value}"
    );
}
