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

    /// Test-only: opens a store database on a file with a real multi-connection
    /// pool so tests can exercise true concurrency.
    ///
    /// [`StoreDatabase::in_memory`] pins its pool to a single connection, which
    /// serializes every "concurrent" task at the pool boundary. A multi-connection
    /// pool cannot be memory-backed instead: SQLite gives every connection its
    /// own private database for `:memory:`, so each pooled connection would see
    /// an empty schema. The caller therefore supplies a file path (typically
    /// inside a `tempfile::TempDir` it keeps alive for the lifetime of the
    /// database) and an explicit pool size.
    ///
    /// Not part of any production path; production store databases are opened
    /// through [`StoreDatabaseManager`].
    pub async fn file_backed_multi_connection(
        store_id: StoreId,
        path: impl AsRef<Path>,
        max_connections: u32,
    ) -> Result<Self> {
        let path = path.as_ref();
        if path.to_string_lossy().trim() == ":memory:" {
            return Err(InfrastructureError::InvalidInput(
                "a multi-connection store database needs a file path; SQLite :memory: databases \
                 are private to each connection"
                    .to_owned(),
            ));
        }
        let options = DatabaseOptions::default().with_max_connections(max_connections);
        let pool = connect_sqlite(path, &options).await?;
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

mod backup;
mod manager;

#[cfg(test)]
mod tests;
