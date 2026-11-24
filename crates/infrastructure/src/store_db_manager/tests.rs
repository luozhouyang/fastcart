use fastcart_kernel::StoreId;

use super::{BackupManifest, StoreDatabase, StoreDatabaseManager};
use crate::database::DatabaseOptions;

fn manager(dir: &tempfile::TempDir) -> StoreDatabaseManager {
    StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    )
}

fn sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    std::path::PathBuf::from(sidecar)
}

#[tokio::test]
async fn stores_get_independent_databases_and_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let first = StoreId::new();
    let second = StoreId::new();

    let db_a = manager.database(&first).await.unwrap();
    let db_b = manager.database(&second).await.unwrap();

    assert_ne!(db_a.store_id(), db_b.store_id());
    assert_ne!(
        manager.database_path(&first).unwrap(),
        manager.database_path(&second).unwrap()
    );

    sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('name', 'a', 0)")
        .execute(db_a.pool())
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
        .fetch_one(db_b.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn migrations_create_operational_indexes() {
    let database = StoreDatabase::in_memory(StoreId::new()).await.unwrap();

    for (index, table) in [
        ("idx_inventory_reservations_order", "inventory_reservations"),
        ("idx_order_items_variant", "order_items"),
        ("idx_cart_items_variant", "cart_items"),
    ] {
        let indexed_table: Option<String> = sqlx::query_scalar(
            "SELECT tbl_name FROM sqlite_master WHERE type = 'index' AND name = ?",
        )
        .bind(index)
        .fetch_optional(database.pool())
        .await
        .unwrap();
        assert_eq!(
            indexed_table.as_deref(),
            Some(table),
            "migration did not create index {index} on {table}"
        );
    }
}

#[tokio::test]
async fn pool_creation_is_lazy_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).with_max_pools(1);
    let first = StoreId::new();
    let second = StoreId::new();

    assert_eq!(manager.open_pool_count().await, 0);
    manager.database(&first).await.unwrap();
    assert_eq!(manager.open_pool_count().await, 1);
    manager.database(&second).await.unwrap();
    assert_eq!(manager.open_pool_count().await, 1);
}

#[tokio::test]
async fn rejects_unsafe_store_keys() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store_id = StoreId::from_string("../../etc/passwd");
    assert!(manager.database_path(&store_id).is_err());
}

#[tokio::test]
async fn backup_and_restore_preserve_data() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    let db = manager.database(&store).await.unwrap();

    sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('name', 'a', 0)")
        .execute(db.pool())
        .await
        .unwrap();

    let backup_path = dir.path().join("backups/store.sqlite");
    let manifest = manager.backup(&store, &backup_path).await.unwrap();
    assert_eq!(manifest.store_id, store.to_string());
    assert!(manifest.bytes > 0);

    sqlx::query("UPDATE settings SET value = 'changed' WHERE key = 'name'")
        .execute(db.pool())
        .await
        .unwrap();

    manager.restore(&store, &backup_path).await.unwrap();
    let restored = manager.database(&store).await.unwrap();
    let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'name'")
        .fetch_one(restored.pool())
        .await
        .unwrap();
    assert_eq!(value, "a");
}

#[tokio::test]
async fn backup_writes_a_manifest_and_verify_accepts_it() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    manager.database(&store).await.unwrap();

    let backup_path = dir.path().join("backups/store.sqlite");
    let manifest = manager.backup(&store, &backup_path).await.unwrap();

    let manifest_path = StoreDatabaseManager::manifest_path(&backup_path);
    assert!(manifest_path.is_file(), "backup manifest is written");
    let loaded: BackupManifest =
        serde_json::from_slice(&tokio::fs::read(&manifest_path).await.unwrap()).unwrap();
    assert_eq!(loaded.store_id, store.to_string());
    assert_eq!(loaded.bytes, manifest.bytes);
    assert_eq!(loaded.sha256, manifest.sha256);

    StoreDatabaseManager::verify_backup(&backup_path)
        .await
        .unwrap();
    let verified = StoreDatabaseManager::verify_backup_with_manifest(&backup_path)
        .await
        .unwrap()
        .expect("the manifest is present");
    assert_eq!(verified.sha256, manifest.sha256);
}

#[tokio::test]
async fn verify_backup_does_not_create_wal_sidecars() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    manager.database(&store).await.unwrap();

    let backup_path = dir.path().join("backups/store.sqlite");
    manager.backup(&store, &backup_path).await.unwrap();
    let wal = sidecar(&backup_path, "-wal");
    let shm = sidecar(&backup_path, "-shm");
    let _ = tokio::fs::remove_file(&wal).await;
    let _ = tokio::fs::remove_file(&shm).await;

    StoreDatabaseManager::verify_backup(&backup_path)
        .await
        .unwrap();

    assert!(
        !tokio::fs::try_exists(&wal).await.unwrap(),
        "verify must not create a -wal file next to the backup"
    );
    assert!(
        !tokio::fs::try_exists(&shm).await.unwrap(),
        "verify must not create a -shm file next to the backup"
    );
}

#[tokio::test]
async fn restore_rejects_a_manifest_from_another_store() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    manager.database(&store).await.unwrap();

    let backup_path = dir.path().join("backups/store.sqlite");
    let manifest = manager.backup(&store, &backup_path).await.unwrap();
    let mut foreign = manifest;
    foreign.store_id = StoreId::new().to_string();
    let manifest_path = StoreDatabaseManager::manifest_path(&backup_path);
    tokio::fs::write(&manifest_path, serde_json::to_vec_pretty(&foreign).unwrap())
        .await
        .unwrap();

    let error = manager.restore(&store, &backup_path).await.unwrap_err();
    assert!(
        error.to_string().contains("belongs to store"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn restore_removes_stale_destination_wal_files() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    let database = manager.database(&store).await.unwrap();

    sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('name', 'original', 0)")
        .execute(database.pool())
        .await
        .unwrap();
    let backup_path = dir.path().join("backups/store.sqlite");
    manager.backup(&store, &backup_path).await.unwrap();

    sqlx::query("UPDATE settings SET value = 'changed' WHERE key = 'name'")
        .execute(database.pool())
        .await
        .unwrap();
    manager.close(&store).await;

    let destination = manager.database_path(&store).unwrap();
    let wal = sidecar(&destination, "-wal");
    let shm = sidecar(&destination, "-shm");
    tokio::fs::write(&wal, b"stale wal").await.unwrap();
    tokio::fs::write(&shm, b"stale shm").await.unwrap();

    manager.restore(&store, &backup_path).await.unwrap();

    assert!(
        !tokio::fs::try_exists(&wal).await.unwrap(),
        "stale destination WAL must be removed"
    );
    assert!(
        !tokio::fs::try_exists(&shm).await.unwrap(),
        "stale destination SHM must be removed"
    );

    let restored = manager.database(&store).await.unwrap();
    let value: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'name'")
        .fetch_one(restored.pool())
        .await
        .unwrap();
    assert_eq!(value, "original");
}

#[tokio::test]
async fn evicting_a_pool_keeps_live_store_handles_usable() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir).with_max_pools(1);
    let first = StoreId::new();
    let second = StoreId::new();

    let held = manager.database(&first).await.unwrap();
    manager.database(&second).await.unwrap();
    assert_eq!(manager.open_pool_count().await, 1);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM settings")
        .fetch_one(held.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('evicted', 'ok', 0)")
        .execute(held.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn immediate_transactions_serialize_concurrent_writers() {
    let dir = tempfile::tempdir().unwrap();
    let manager = StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(8),
    );
    let database = manager.database(&StoreId::new()).await.unwrap();
    sqlx::query(
            "CREATE TABLE concurrency_probe (id INTEGER PRIMARY KEY AUTOINCREMENT, value INTEGER NOT NULL)",
        )
        .execute(database.pool())
        .await
        .unwrap();
    let database = std::sync::Arc::new(database);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let database = database.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = database.begin().await.unwrap();
            let value: i64 =
                sqlx::query_scalar("SELECT COALESCE(MAX(value), 0) + 1 FROM concurrency_probe")
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
            sqlx::query("INSERT INTO concurrency_probe (value) VALUES (?)")
                .bind(value)
                .execute(&mut *tx)
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            tx.commit().await.unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let values: Vec<i64> = sqlx::query_scalar("SELECT value FROM concurrency_probe ORDER BY value")
        .fetch_all(database.pool())
        .await
        .unwrap();
    assert_eq!(values, (1..=8).collect::<Vec<_>>());
}
