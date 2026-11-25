//! Plugin state retention.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use fastcart_plugin::dispatch::{prune_dispatch_state, prune_expired_kv};

const NOW: i64 = 1_000_000_000;
const CUTOFF: i64 = NOW - 30 * 86_400;

#[tokio::test]
async fn expired_kv_rows_are_pruned() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    for (key, expires_at) in [
        ("live", None),
        ("expired", Some(NOW - 1)),
        ("future", Some(NOW + 60)),
    ] {
        sqlx::query(
            "INSERT INTO plugin_kv (plugin_id, namespace, key, value, expires_at, updated_at) \
             VALUES ('demo', 'ns', ?, X'01', ?, 1)",
        )
        .bind(key)
        .bind(expires_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    assert_eq!(prune_expired_kv(&db, NOW).await.unwrap(), 1);
    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM plugin_kv ORDER BY key")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert_eq!(keys, ["future", "live"]);
}

#[tokio::test]
async fn terminal_dispatch_rows_are_pruned_but_pending_work_survives() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    for (id, status, created_at) in [
        ("done-old", "done", CUTOFF - 1),
        ("dead-old", "dead", CUTOFF - 1),
        ("done-new", "done", CUTOFF + 1),
        ("pending-old", "pending", CUTOFF - 1),
        ("running-old", "running", CUTOFF - 1),
    ] {
        sqlx::query(
            "INSERT INTO plugin_deliveries \
             (id, plugin_id, event_id, event_type, payload, status, attempts, available_at, created_at) \
             VALUES (?, 'demo', ?, 'order.placed', '{}', ?, 0, 1, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(status)
        .bind(created_at)
        .execute(db.pool())
        .await
        .unwrap();
    }
    for (id, status, updated_at) in [
        ("job-done-old", "completed", CUTOFF - 1),
        ("job-failed-old", "failed", CUTOFF - 1),
        ("job-done-new", "completed", CUTOFF + 1),
        ("job-pending-old", "pending", CUTOFF - 1),
    ] {
        sqlx::query(
            "INSERT INTO plugin_jobs \
             (id, plugin_id, name, payload, status, attempts, available_at, created_at, updated_at) \
             VALUES (?, 'demo', 'sync', '{}', ?, 0, 1, 1, ?)",
        )
        .bind(id)
        .bind(status)
        .bind(updated_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    let (deliveries, jobs) = prune_dispatch_state(&db, NOW).await.unwrap();
    assert_eq!((deliveries, jobs), (2, 2));

    let delivery_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM plugin_deliveries ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(delivery_ids, ["done-new", "pending-old", "running-old"]);
    let job_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM plugin_jobs ORDER BY id")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert_eq!(job_ids, ["job-done-new", "job-pending-old"]);
}
