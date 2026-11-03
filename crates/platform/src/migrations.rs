//! Platform database bootstrap.

use std::path::Path;

use fastcart_infrastructure::DatabaseOptions;
use fastcart_infrastructure::database::{connect_memory, connect_sqlite};
use sqlx::SqlitePool;
use sqlx::migrate::Migrator;

use crate::{PlatformError, PlatformResult};

/// Embedded platform migrations.
pub static PLATFORM_MIGRATOR: Migrator = sqlx::migrate!("../../migrations/platform");

/// Opens and migrates the platform database.
pub async fn open_platform_database(
    path: impl AsRef<Path>,
    options: &DatabaseOptions,
) -> PlatformResult<SqlitePool> {
    let pool = connect_sqlite(path, options).await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

/// Opens a private in-memory platform database for tests.
pub async fn open_memory_database() -> PlatformResult<SqlitePool> {
    let pool = connect_memory().await?;
    run_migrations(&pool).await?;
    Ok(pool)
}

/// Runs pending platform migrations.
pub async fn run_migrations(pool: &SqlitePool) -> PlatformResult<()> {
    PLATFORM_MIGRATOR
        .run(pool)
        .await
        .map_err(|error| PlatformError::Database(format!("platform migration failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::open_memory_database;

    #[tokio::test]
    async fn migrations_seed_roles_and_permissions() {
        let pool = open_memory_database().await.unwrap();
        let roles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles")
            .fetch_one(&pool)
            .await
            .unwrap();
        let permissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM permissions")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(roles, 7);
        assert!(permissions >= 17);
    }
}
