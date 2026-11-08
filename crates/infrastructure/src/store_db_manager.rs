//! Per-store SQLite databases: lazy pools, migrations, health and backups.
//!
//! Store databases are isolated by construction: every service receives a
//! [`StoreDatabase`] handle bound to exactly one store identifier. Repository
//! code never resolves filesystem paths itself.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};

use fastcart_kernel::{StoreId, now_timestamp};
use sha2::{Digest, Sha256};
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
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

/// Migration state for one store database.
///
/// This is an additive operational health signal. It never changes how
/// migrations are applied; it only reports what the embedded migrator expects
/// against what has been recorded in `_sqlx_migrations`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigrationHealth {
    /// Number of successfully applied migrations.
    pub applied: i64,
    /// Highest embedded migration version.
    pub latest: i64,
    /// Number of embedded migrations that have not been applied yet.
    pub pending: i64,
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

    /// Reports the applied migration state for a store database.
    ///
    /// The store database is opened and migrated lazily first, so a healthy
    /// store reports `pending = 0`.
    pub async fn migration_health(&self, store_id: &StoreId) -> Result<MigrationHealth> {
        let database = self.database(store_id).await?;
        let applied_versions: Vec<i64> = sqlx::query_scalar(
            "SELECT version FROM _sqlx_migrations WHERE success = 1 ORDER BY version",
        )
        .fetch_all(database.pool())
        .await?;
        let expected: Vec<i64> = STORE_MIGRATOR
            .iter()
            .map(|migration| migration.version)
            .collect();
        let pending = expected
            .iter()
            .filter(|version| !applied_versions.contains(version))
            .count();
        let latest = expected.iter().copied().max().unwrap_or(0);
        Ok(MigrationHealth {
            applied: i64::try_from(applied_versions.len()).unwrap_or(i64::MAX),
            latest,
            pending: i64::try_from(pending).unwrap_or(i64::MAX),
        })
    }

    /// Creates a consistent SQLite backup without copying an active WAL file.
    ///
    /// `VACUUM INTO` produces a transactionally consistent snapshot regardless
    /// of concurrent readers and writers. A sibling `<backup>.manifest.json`
    /// manifest is written after the checksum is computed.
    pub async fn backup(
        &self,
        store_id: &StoreId,
        destination: impl AsRef<Path>,
    ) -> Result<BackupManifest> {
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        if destination.exists() {
            tokio::fs::remove_file(destination).await?;
        }

        let database = self.database(store_id).await?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination.to_string_lossy().as_ref())
            .execute(database.pool())
            .await?;

        let (bytes, sha256) = Self::sha256_file(destination).await?;
        let manifest = BackupManifest {
            store_id: store_id.to_string(),
            created_at: now_timestamp(),
            file: destination.to_string_lossy().into_owned(),
            bytes,
            sha256,
        };
        let manifest_path = Self::manifest_path(destination);
        let encoded = serde_json::to_vec_pretty(&manifest).map_err(|error| {
            InfrastructureError::InvalidInput(format!(
                "failed to serialize backup manifest: {error}"
            ))
        })?;
        tokio::fs::write(&manifest_path, encoded).await?;
        Ok(manifest)
    }

    /// Returns the sibling manifest path for a backup file.
    ///
    /// The manifest is appended to the file name, so a backup at
    /// `store.sqlite` has a manifest at `store.sqlite.manifest.json`.
    #[must_use]
    pub fn manifest_path(backup: impl AsRef<Path>) -> PathBuf {
        let mut path = backup.as_ref().as_os_str().to_owned();
        path.push(".manifest.json");
        PathBuf::from(path)
    }

    /// Loads the sibling manifest for a backup file, when one exists.
    pub async fn load_backup_manifest(backup: impl AsRef<Path>) -> Result<Option<BackupManifest>> {
        let manifest_path = Self::manifest_path(backup);
        match tokio::fs::read(&manifest_path).await {
            Ok(bytes) => {
                let manifest = serde_json::from_slice(&bytes).map_err(|error| {
                    InfrastructureError::InvalidInput(format!(
                        "invalid backup manifest {}: {error}",
                        manifest_path.display()
                    ))
                })?;
                Ok(Some(manifest))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Verifies a backup file without creating or modifying it.
    ///
    /// The file is opened through a read-only SQLite connection. No
    /// create-if-missing, WAL or journal pragmas are applied, so verification
    /// never creates the database or `-wal`/`-shm` sidecars.
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
        let connect_options = SqliteConnectOptions::from_str(&path.to_string_lossy())
            .map_err(|error| InfrastructureError::InvalidInput(error.to_string()))?
            .read_only(true)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options)
            .await?;
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

    /// Verifies a backup file and, when present, its sibling manifest.
    ///
    /// The checksum and byte count in the manifest must match the backup.
    /// Returns the manifest that was checked, if one exists.
    pub async fn verify_backup_with_manifest(
        backup: impl AsRef<Path>,
    ) -> Result<Option<BackupManifest>> {
        let backup = backup.as_ref();
        Self::verify_backup(backup).await?;
        let manifest = Self::load_backup_manifest(backup).await?;
        if let Some(manifest) = manifest.as_ref() {
            Self::verify_manifest(backup, manifest).await?;
        }
        Ok(manifest)
    }

    async fn verify_manifest(path: &Path, manifest: &BackupManifest) -> Result<()> {
        let (bytes, sha256) = Self::sha256_file(path).await?;
        if bytes != manifest.bytes {
            return Err(InfrastructureError::Integrity(format!(
                "backup size {bytes} does not match manifest size {}",
                manifest.bytes
            )));
        }
        if !sha256.eq_ignore_ascii_case(&manifest.sha256) {
            return Err(InfrastructureError::Integrity(
                "backup checksum does not match its manifest".to_owned(),
            ));
        }
        Ok(())
    }

    async fn sha256_file(path: &Path) -> Result<(u64, String)> {
        let mut file = tokio::fs::File::open(path).await?;
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
        Ok((bytes, hex::encode(hasher.finalize())))
    }

    /// Restores a verified backup into a store database file.
    ///
    /// When a sibling manifest exists, the backup must belong to `store_id`
    /// and match the manifest checksum. The store pool is closed first, and
    /// destination `-wal`/`-shm` files are removed so a stale write-ahead log
    /// cannot be replayed over the restored database. The restored database is
    /// left closed and reopens lazily on the next access.
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
        let manifest = Self::load_backup_manifest(backup).await?;
        if let Some(manifest) = manifest.as_ref()
            && manifest.store_id != store_id.as_str()
        {
            return Err(InfrastructureError::InvalidInput(format!(
                "backup belongs to store {} but restore targets {store_id}",
                manifest.store_id
            )));
        }
        Self::verify_backup(&canonical_backup).await?;
        if let Some(manifest) = manifest.as_ref() {
            Self::verify_manifest(&canonical_backup, manifest).await?;
        }
        self.close(store_id).await;
        Self::remove_sidecars(&destination).await?;
        if destination.exists() {
            let previous = destination.with_extension("sqlite.pre-restore");
            tokio::fs::copy(&destination, &previous).await?;
        }
        tokio::fs::copy(&canonical_backup, &destination).await?;
        Self::remove_sidecars(&destination).await?;
        self.migrated.write().await.remove(store_id.as_str());
        Ok(())
    }

    async fn remove_sidecars(path: &Path) -> Result<()> {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            match tokio::fs::remove_file(PathBuf::from(sidecar)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
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
            // Eviction only forgets the pool. Calls already holding a cloned
            // `StoreDatabase` keep a live `SqlitePool` handle, so force-closing
            // here would make their later queries fail. The pool and its
            // connections close when the last handle is dropped.
            let _ = self.pools.write().await.remove(&victim);
        }
    }
}

#[cfg(test)]
mod tests {
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
        let values: Vec<i64> =
            sqlx::query_scalar("SELECT value FROM concurrency_probe ORDER BY value")
                .fetch_all(database.pool())
                .await
                .unwrap();
        assert_eq!(values, (1..=8).collect::<Vec<_>>());
    }
}
