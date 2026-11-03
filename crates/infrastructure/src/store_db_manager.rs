//! Per-store SQLite databases: lazy pools, migrations, health and backups.
//!
//! Store databases are isolated by construction: every service receives a
//! [`StoreDatabase`] handle bound to exactly one store identifier. Repository
//! code never resolves filesystem paths itself.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use fastcart_kernel::{StoreId, now_timestamp};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::{SqlitePool, Transaction};
use tokio::sync::RwLock;

use crate::database::{
    DatabaseHealth, DatabaseOptions, checkpoint, connect_sqlite, health_check, verify_integrity,
};
use crate::{InfrastructureError, Result};

/// Embedded store migrations. Every store database tracks its own version.
pub static STORE_MIGRATOR: Migrator = sqlx::migrate!("../../migrations/store");

/// A handle to one store's database.
#[derive(Clone, Debug)]
pub struct StoreDatabase {
    store_id: StoreId,
    pool: SqlitePool,
}

impl StoreDatabase {
    /// Wraps a pool already bound to a store.
    #[must_use]
    pub fn new(store_id: StoreId, pool: SqlitePool) -> Self {
        Self { store_id, pool }
    }

    /// Creates an isolated in-memory store database with migrations applied.
    ///
    /// Intended for module tests: each call gets its own SQLite database.
    pub async fn in_memory(store_id: StoreId) -> Result<Self> {
        let pool = crate::database::connect_memory().await?;
        STORE_MIGRATOR
            .run(&pool)
            .await
            .map_err(|error| InfrastructureError::Migration(error.to_string()))?;
        Ok(Self::new(store_id, pool))
    }

    /// Returns the owning store identifier.
    #[must_use]
    pub fn store_id(&self) -> &StoreId {
        &self.store_id
    }

    /// Returns the underlying pool.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Begins an immediate write transaction.
    ///
    /// `BEGIN IMMEDIATE` takes the SQLite write lock up front, avoiding
    /// `SQLITE_BUSY_SNAPSHOT` failures when concurrent deferred transactions
    /// upgrade from read to write.
    pub async fn begin(&self) -> Result<Transaction<'_, sqlx::Sqlite>> {
        Ok(self.pool.begin_with("BEGIN IMMEDIATE").await?)
    }

    /// Runs a quick health check.
    pub async fn health_check(&self) -> Result<DatabaseHealth> {
        health_check(&self.pool, true).await
    }
}

/// Metadata written alongside every backup.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupManifest {
    /// Store identifier.
    pub store_id: String,
    /// Backup timestamp in Unix seconds.
    pub created_at: i64,
    /// Absolute or relative backup file path.
    pub file: String,
    /// File size in bytes.
    pub bytes: u64,
    /// SHA-256 checksum of the backup file.
    pub sha256: String,
}

#[derive(Debug)]
struct PoolEntry {
    pool: SqlitePool,
    last_used: AtomicI64,
}

/// Owns the bounded set of per-store connection pools.
#[derive(Debug)]
pub struct StoreDatabaseManager {
    stores_dir: PathBuf,
    options: DatabaseOptions,
    max_pools: usize,
    pools: RwLock<HashMap<String, PoolEntry>>,
    migrated: RwLock<HashSet<String>>,
}

impl StoreDatabaseManager {
    /// Creates a manager rooted at the configured stores directory.
    #[must_use]
    pub fn new(stores_dir: impl Into<PathBuf>, options: DatabaseOptions) -> Self {
        Self {
            stores_dir: stores_dir.into(),
            options,
            max_pools: 64,
            pools: RwLock::new(HashMap::new()),
            migrated: RwLock::new(HashSet::new()),
        }
    }

    /// Sets the maximum number of simultaneously open store pools.
    #[must_use]
    pub fn with_max_pools(mut self, max_pools: usize) -> Self {
        self.max_pools = max_pools.max(1);
        self
    }

    /// Returns the stores directory.
    #[must_use]
    pub fn stores_dir(&self) -> &Path {
        &self.stores_dir
    }

    /// Returns the database path for a store after validating the identifier.
    pub fn database_path(&self, store_id: &StoreId) -> Result<PathBuf> {
        let key = store_id.as_str();
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(InfrastructureError::InvalidInput(format!(
                "unsafe store database key: {key}"
            )));
        }
        Ok(self.stores_dir.join(format!("{key}.sqlite")))
    }

    /// Returns a store database handle, opening and migrating it lazily.
    pub async fn database(&self, store_id: &StoreId) -> Result<StoreDatabase> {
        let key = store_id.as_str().to_owned();
        if let Some(pool) = self.existing_pool(&key).await {
            return Ok(StoreDatabase::new(store_id.clone(), pool));
        }

        let path = self.database_path(store_id)?;
        let pool = connect_sqlite(&path, &self.options).await?;
        if let Err(error) = STORE_MIGRATOR.run(&pool).await {
            pool.close().await;
            return Err(InfrastructureError::Migration(error.to_string()));
        }

        let mut pools = self.pools.write().await;
        if let Some(entry) = pools.get(&key) {
            let existing = entry.pool.clone();
            pool.close().await;
            return Ok(StoreDatabase::new(store_id.clone(), existing));
        }
        pools.insert(
            key.clone(),
            PoolEntry {
                pool: pool.clone(),
                last_used: AtomicI64::new(now_timestamp()),
            },
        );
        drop(pools);

        self.mark_migrated(&key).await;
        self.evict_excess(&key).await;
        Ok(StoreDatabase::new(store_id.clone(), pool))
    }

    async fn existing_pool(&self, key: &str) -> Option<SqlitePool> {
        let pools = self.pools.read().await;
        let entry = pools.get(key)?;
        entry.last_used.store(now_timestamp(), Ordering::Relaxed);
        Some(entry.pool.clone())
    }

    async fn mark_migrated(&self, key: &str) {
        self.migrated.write().await.insert(key.to_owned());
    }

    /// Runs pending migrations for a store. Safe to call repeatedly.
    pub async fn migrate(&self, store_id: &StoreId) -> Result<()> {
        let key = store_id.as_str().to_owned();
        if self.migrated.read().await.contains(&key) {
            return Ok(());
        }
        let database = self.database(store_id).await?;
        STORE_MIGRATOR
            .run(database.pool())
            .await
            .map_err(|error| InfrastructureError::Migration(error.to_string()))?;
        self.mark_migrated(&key).await;
        Ok(())
    }

    /// Runs a health check against a store database.
    pub async fn health_check(&self, store_id: &StoreId) -> Result<DatabaseHealth> {
        let database = self.database(store_id).await?;
        health_check(database.pool(), true).await
    }

    /// Verifies database integrity.
    pub async fn verify_integrity(&self, store_id: &StoreId) -> Result<()> {
        let database = self.database(store_id).await?;
        verify_integrity(database.pool()).await
    }

    /// Creates a consistent SQLite backup without copying an active WAL file.
    ///
    /// `VACUUM INTO` produces a transactionally consistent snapshot regardless
    /// of concurrent readers and writers.
    pub async fn backup(
        &self,
        store_id: &StoreId,
        destination: impl AsRef<Path>,
    ) -> Result<BackupManifest> {
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        if destination.exists() {
            tokio::fs::remove_file(destination).await?;
        }

        let database = self.database(store_id).await?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination.to_string_lossy().as_ref())
            .execute(database.pool())
            .await?;

        let mut file = tokio::fs::File::open(destination).await?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = tokio::io::AsyncReadExt::read(&mut file, &mut buffer).await?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            bytes += u64::try_from(read).unwrap_or(u64::MAX);
        }

        Ok(BackupManifest {
            store_id: store_id.to_string(),
            created_at: now_timestamp(),
            file: destination.to_string_lossy().into_owned(),
            bytes,
            sha256: hex::encode(hasher.finalize()),
        })
    }

    /// Verifies a backup file without creating or modifying it.
    pub async fn verify_backup(path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path).await.map_err(|error| {
            InfrastructureError::InvalidInput(format!("backup is not readable: {error}"))
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(InfrastructureError::Integrity(
                "backup file is empty or not a regular file".to_owned(),
            ));
        }
        let pool = connect_sqlite(path, &DatabaseOptions::default()).await?;
        let result = async {
            verify_integrity(&pool).await?;
            let objects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
                    .fetch_one(&pool)
                    .await?;
            if objects == 0 {
                return Err(InfrastructureError::Integrity(
                    "backup contains no schema objects".to_owned(),
                ));
            }
            Ok(())
        }
        .await;
        pool.close().await;
        result
    }

    /// Restores a verified backup into a store database file.
    ///
    /// The store must be closed first; callers are expected to suspend the
    /// store before invoking this operation.
    pub async fn restore(&self, store_id: &StoreId, backup: impl AsRef<Path>) -> Result<()> {
        let backup = backup.as_ref();
        let destination = self.database_path(store_id)?;
        let canonical_backup = tokio::fs::canonicalize(backup).await?;
        let canonical_destination = if destination.exists() {
            tokio::fs::canonicalize(&destination).await?
        } else {
            destination.clone()
        };
        if canonical_backup == canonical_destination {
            return Err(InfrastructureError::InvalidInput(
                "backup and restore paths must differ".to_owned(),
            ));
        }
        Self::verify_backup(&canonical_backup).await?;
        self.close(store_id).await;
        if destination.exists() {
            let previous = destination.with_extension("sqlite.pre-restore");
            tokio::fs::copy(&destination, &previous).await?;
        }
        tokio::fs::copy(&canonical_backup, &destination).await?;
        self.migrated.write().await.remove(store_id.as_str());
        self.database(store_id).await?;
        Ok(())
    }

    /// Closes a store pool after checkpointing WAL state.
    pub async fn close(&self, store_id: &StoreId) {
        let removed = self.pools.write().await.remove(store_id.as_str());
        if let Some(entry) = removed {
            let _ = checkpoint(&entry.pool).await;
            entry.pool.close().await;
        }
    }

    /// Closes every open store pool.
    pub async fn close_all(&self) {
        let entries: Vec<PoolEntry> = {
            let mut pools = self.pools.write().await;
            pools.drain().map(|(_, entry)| entry).collect()
        };
        for entry in entries {
            let _ = checkpoint(&entry.pool).await;
            entry.pool.close().await;
        }
    }

    /// Number of currently open pools.
    pub async fn open_pool_count(&self) -> usize {
        self.pools.read().await.len()
    }

    async fn evict_excess(&self, keep: &str) {
        loop {
            let victim = {
                let pools = self.pools.read().await;
                if pools.len() <= self.max_pools {
                    break;
                }
                pools
                    .iter()
                    .filter(|(key, _)| key.as_str() != keep)
                    .min_by_key(|(_, entry)| entry.last_used.load(Ordering::Relaxed))
                    .map(|(key, _)| key.clone())
            };
            let Some(victim) = victim else {
                break;
            };
            if let Some(entry) = self.pools.write().await.remove(&victim) {
                tokio::time::sleep(Duration::from_millis(0)).await;
                let _ = checkpoint(&entry.pool).await;
                entry.pool.close().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::StoreId;

    use super::StoreDatabaseManager;
    use crate::database::DatabaseOptions;

    fn manager(dir: &tempfile::TempDir) -> StoreDatabaseManager {
        StoreDatabaseManager::new(
            dir.path().join("stores"),
            DatabaseOptions::default().with_max_connections(2),
        )
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
        let values: Vec<i64> =
            sqlx::query_scalar("SELECT value FROM concurrency_probe ORDER BY value")
                .fetch_all(database.pool())
                .await
                .unwrap();
        assert_eq!(values, (1..=8).collect::<Vec<_>>());
    }
}
