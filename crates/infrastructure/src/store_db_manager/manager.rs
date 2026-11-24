//! Manager construction, pool lifecycle and migrations.

use super::*;

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
}

impl StoreDatabaseManager {
    /// Sets the maximum number of simultaneously open store pools.
    #[must_use]
    pub fn with_max_pools(mut self, max_pools: usize) -> Self {
        self.max_pools = max_pools.max(1);
        self
    }
}

impl StoreDatabaseManager {
    /// Returns the stores directory.
    #[must_use]
    pub fn stores_dir(&self) -> &Path {
        &self.stores_dir
    }
}

impl StoreDatabaseManager {
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
}

impl StoreDatabaseManager {
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
}

impl StoreDatabaseManager {
    async fn existing_pool(&self, key: &str) -> Option<SqlitePool> {
        let pools = self.pools.read().await;
        let entry = pools.get(key)?;
        entry.last_used.store(now_timestamp(), Ordering::Relaxed);
        Some(entry.pool.clone())
    }
}

impl StoreDatabaseManager {
    async fn mark_migrated(&self, key: &str) {
        self.migrated.write().await.insert(key.to_owned());
    }
}

impl StoreDatabaseManager {
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
}

impl StoreDatabaseManager {
    /// Runs a health check against a store database.
    pub async fn health_check(&self, store_id: &StoreId) -> Result<DatabaseHealth> {
        let database = self.database(store_id).await?;
        health_check(database.pool(), true).await
    }
}

impl StoreDatabaseManager {
    /// Verifies database integrity.
    pub async fn verify_integrity(&self, store_id: &StoreId) -> Result<()> {
        let database = self.database(store_id).await?;
        verify_integrity(database.pool()).await
    }
}

impl StoreDatabaseManager {
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
}

impl StoreDatabaseManager {
    /// Closes a store pool after checkpointing WAL state.
    pub async fn close(&self, store_id: &StoreId) {
        let removed = self.pools.write().await.remove(store_id.as_str());
        if let Some(entry) = removed {
            let _ = checkpoint(&entry.pool).await;
            entry.pool.close().await;
        }
    }
}

impl StoreDatabaseManager {
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
}

impl StoreDatabaseManager {
    /// Number of currently open pools.
    pub async fn open_pool_count(&self) -> usize {
        self.pools.read().await.len()
    }
}

impl StoreDatabaseManager {
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
