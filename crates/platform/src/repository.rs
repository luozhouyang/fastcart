//! Shared platform database handle.

use std::path::Path;

use fastcart_infrastructure::DatabaseOptions;
use sqlx::SqlitePool;

use crate::PlatformResult;
use crate::migrations::{open_memory_database, open_platform_database};

/// Handle to the global platform SQLite database.
#[derive(Clone, Debug)]
pub struct PlatformStore {
    pool: SqlitePool,
    pub(crate) audit_actor: Option<String>,
}

impl PlatformStore {
    /// Wraps an already-migrated pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            audit_actor: None,
        }
    }

    /// Sets the acting user recorded on platform mutations.
    ///
    /// The actor is carried on the clone returned by the builder, so a shared
    /// handle is never mutated.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit_actor = actor_id;
        self
    }

    /// Opens and migrates the platform database at `path`.
    pub async fn open(path: impl AsRef<Path>, options: &DatabaseOptions) -> PlatformResult<Self> {
        Ok(Self::new(open_platform_database(path, options).await?))
    }

    /// Opens a private in-memory database for tests.
    pub async fn in_memory() -> PlatformResult<Self> {
        Ok(Self::new(open_memory_database().await?))
    }

    /// Returns the underlying pool.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Runs a lightweight connectivity check.
    pub async fn ping(&self) -> PlatformResult<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }
}
