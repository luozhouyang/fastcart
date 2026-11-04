//! SQLite connection configuration, health checks and pool helpers.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

use crate::InfrastructureError;

/// Connection pool and SQLite pragma settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatabaseOptions {
    /// Maximum connections per database pool.
    pub max_connections: u32,
    /// SQLite busy timeout in milliseconds.
    pub busy_timeout_ms: u64,
    /// Whether WAL journaling should be enabled.
    pub wal: bool,
}

impl Default for DatabaseOptions {
    fn default() -> Self {
        Self {
            max_connections: 5,
            busy_timeout_ms: 5_000,
            wal: true,
        }
    }
}

impl DatabaseOptions {
    /// Creates options with an explicit pool size.
    #[must_use]
    pub fn with_max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections.max(1);
        self
    }
}

/// Database health snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseHealth {
    /// Whether the database accepted a query.
    pub healthy: bool,
    /// SQLite integrity check result, when requested.
    pub integrity: Option<String>,
    /// SQLite library version.
    pub version: Option<String>,
}

/// Opens a SQLite pool, creating the file and parent directory when needed.
pub async fn connect_sqlite(
    path: impl AsRef<Path>,
    options: &DatabaseOptions,
) -> crate::Result<SqlitePool> {
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent).await?;
    }

    let connect_options = sqlite_options(path.to_string_lossy().as_ref(), options)?;
    let pool = SqlitePoolOptions::new()
        .max_connections(options.max_connections)
        .acquire_timeout(Duration::from_millis(options.busy_timeout_ms.max(1_000)))
        .connect_with(connect_options)
        .await?;
    Ok(pool)
}

/// Opens a private in-memory SQLite pool. Intended for tests.
pub async fn connect_memory() -> crate::Result<SqlitePool> {
    let options = DatabaseOptions::default().with_max_connections(1);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlite_options(":memory:", &options)?)
        .await?;
    Ok(pool)
}

fn sqlite_options(
    filename: &str,
    options: &DatabaseOptions,
) -> crate::Result<SqliteConnectOptions> {
    let mut connect_options = SqliteConnectOptions::from_str(filename)
        .map_err(|error| InfrastructureError::InvalidInput(error.to_string()))?
        .create_if_missing(filename != ":memory:")
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(options.busy_timeout_ms))
        .synchronous(SqliteSynchronous::Normal);

    if options.wal && filename != ":memory:" {
        connect_options = connect_options.journal_mode(SqliteJournalMode::Wal);
    }

    Ok(connect_options)
}

/// Runs a quick connectivity and integrity check.
pub async fn health_check(
    pool: &SqlitePool,
    check_integrity: bool,
) -> crate::Result<DatabaseHealth> {
    let version: Option<String> = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_optional(pool)
        .await?;

    let integrity = if check_integrity {
        let row = sqlx::query("PRAGMA integrity_check")
            .fetch_one(pool)
            .await?;
        let value: String = row.try_get(0)?;
        Some(value)
    } else {
        None
    };

    Ok(DatabaseHealth {
        healthy: true,
        integrity,
        version,
    })
}

/// Runs the integrity check and returns an error unless SQLite reports `ok`.
pub async fn verify_integrity(pool: &SqlitePool) -> crate::Result<()> {
    let row = sqlx::query("PRAGMA integrity_check")
        .fetch_one(pool)
        .await?;
    let value: String = row.try_get(0)?;
    if value.eq_ignore_ascii_case("ok") {
        Ok(())
    } else {
        Err(InfrastructureError::Integrity(value))
    }
}

/// Forces a WAL checkpoint. Returns the checkpoint result row.
pub async fn checkpoint(pool: &SqlitePool) -> crate::Result<()> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{connect_memory, health_check, verify_integrity};

    #[tokio::test]
    async fn memory_database_reports_healthy() {
        let pool = connect_memory().await.unwrap();
        let health = health_check(&pool, true).await.unwrap();

        assert!(health.healthy);
        assert_eq!(health.integrity.as_deref(), Some("ok"));
        verify_integrity(&pool).await.unwrap();
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let pool = connect_memory().await.unwrap();
        let result = sqlx::query("CREATE TABLE parent (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await;
        assert!(result.is_ok());
        let result = sqlx::query("CREATE TABLE child (parent_id TEXT REFERENCES parent(id))")
            .execute(&pool)
            .await;
        assert!(result.is_ok());
        let insert = sqlx::query("INSERT INTO child (parent_id) VALUES ('missing')")
            .execute(&pool)
            .await;
        assert!(insert.is_err());
    }
}
