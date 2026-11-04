//! Backup, restore, integrity and migration verification.
//!
//! These tests use the real embedded store migrations and a filesystem-backed
//! SQLite database, so they verify the same `VACUUM INTO` path used in
//! production rather than an in-memory approximation.

use fastcart_infrastructure::store_db_manager::STORE_MIGRATOR;
use fastcart_infrastructure::{DatabaseOptions, StoreDatabaseManager};
use fastcart_kernel::StoreId;

fn manager(dir: &tempfile::TempDir) -> StoreDatabaseManager {
    StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    )
}

#[tokio::test]
async fn backup_restore_preserves_data_integrity_and_migration_state() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(&dir);
    let store = StoreId::new();
    let database = manager.database(&store).await.unwrap();

    sqlx::query("INSERT INTO settings (key, value, updated_at) VALUES ('store.name', 'Alpha', 0)")
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sales_channels (id, name, description, is_disabled, metadata, created_at, updated_at)
         VALUES ('sc_backup', 'Backup channel', NULL, 0, '{}', 0, 0)",
    )
    .execute(database.pool())
    .await
    .unwrap();

    let health_before = manager.migration_health(&store).await.unwrap();
    assert!(health_before.applied > 0, "the store database is migrated");
    assert_eq!(
        health_before.pending, 0,
        "no embedded migrations are pending"
    );
    assert_eq!(
        health_before.latest,
        STORE_MIGRATOR
            .iter()
            .map(|migration| migration.version)
            .max()
            .unwrap_or(0)
    );
    manager.verify_integrity(&store).await.unwrap();

    let backup_path = dir.path().join("backups/alpha.sqlite");
    let manifest = manager.backup(&store, &backup_path).await.unwrap();
    assert_eq!(manifest.store_id, store.to_string());
    assert!(manifest.bytes > 0);
    assert_eq!(manifest.sha256.len(), 64);
    assert!(backup_path.is_file());

    // A backup is independently verifiable before it is restored.
    StoreDatabaseManager::verify_backup(&backup_path)
        .await
        .unwrap();

    // Mutate and delete the live data, then restore the earlier snapshot.
    sqlx::query("UPDATE settings SET value = 'Changed' WHERE key = 'store.name'")
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM sales_channels WHERE id = 'sc_backup'")
        .execute(database.pool())
        .await
        .unwrap();

    manager.restore(&store, &backup_path).await.unwrap();

    let restored = manager.database(&store).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'store.name'")
        .fetch_one(restored.pool())
        .await
        .unwrap();
    assert_eq!(name, "Alpha");

    let channel_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels WHERE id = 'sc_backup'")
            .fetch_one(restored.pool())
            .await
            .unwrap();
    assert_eq!(channel_count, 1);

    manager.verify_integrity(&store).await.unwrap();
    let health_after = manager.migration_health(&store).await.unwrap();
    assert_eq!(health_after, health_before);
}

#[tokio::test]
async fn verify_backup_rejects_empty_and_non_database_files() {
    let dir = tempfile::tempdir().unwrap();

    let empty = dir.path().join("empty.sqlite");
    tokio::fs::write(&empty, b"").await.unwrap();
    assert!(StoreDatabaseManager::verify_backup(&empty).await.is_err());

    let text = dir.path().join("not-a-database.sqlite");
    tokio::fs::write(&text, b"this is not a sqlite database")
        .await
        .unwrap();
    assert!(StoreDatabaseManager::verify_backup(&text).await.is_err());

    assert!(
        StoreDatabaseManager::verify_backup(dir.path().join("missing.sqlite"))
            .await
            .is_err()
    );
}
