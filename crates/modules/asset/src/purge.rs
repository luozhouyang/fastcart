//! Durable asset blob purge obligations.
//!
//! A successful soft delete commits a purge row in the same transaction as the
//! metadata change. Delivery to object storage is attempted immediately by the
//! API route; maintenance retries any row that is still pending.

use fastcart_kernel::now_timestamp;
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};

/// A persisted blob purge obligation.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct AssetBlobPurgeRecord {
    /// Purge identifier.
    pub id: String,
    /// Storage key of the object to delete.
    pub storage_key: String,
    /// Failed deletion attempts.
    pub attempts: i64,
    /// Last deletion error.
    pub last_error: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Purge timestamp, null while pending.
    pub processed_at: Option<i64>,
}

/// Inserts a purge obligation using the caller's transaction or pool.
///
/// Passing the asset delete transaction guarantees the obligation commits
/// atomically with the soft delete. Returns the generated identifier so the
/// caller can mark immediate delivery without reading the row back.
pub async fn insert_asset_blob_purge<'e, E>(
    executor: E,
    storage_key: &str,
) -> Result<String, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id = format!("asset_purge_{}", uuid::Uuid::now_v7().simple());
    sqlx::query(
        "INSERT INTO asset_blob_purges (id, storage_key, attempts, last_error, created_at)
         VALUES (?, ?, 0, NULL, ?)",
    )
    .bind(&id)
    .bind(storage_key)
    .bind(now_timestamp())
    .execute(executor)
    .await?;
    Ok(id)
}

/// Fetches blob purges that have not yet reached object storage.
pub async fn fetch_pending_asset_purges(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<AssetBlobPurgeRecord>, sqlx::Error> {
    sqlx::query_as::<_, AssetBlobPurgeRecord>(
        "SELECT id, storage_key, attempts, last_error, created_at, processed_at
         FROM asset_blob_purges
         WHERE processed_at IS NULL
         ORDER BY created_at ASC, id ASC
         LIMIT ?",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
}

/// Fetches the pending purge for one storage key, if any.
///
/// Asset storage keys are unique, so a key has at most one live obligation.
pub async fn fetch_pending_asset_purge_for_key(
    pool: &SqlitePool,
    storage_key: &str,
) -> Result<Option<AssetBlobPurgeRecord>, sqlx::Error> {
    sqlx::query_as::<_, AssetBlobPurgeRecord>(
        "SELECT id, storage_key, attempts, last_error, created_at, processed_at
         FROM asset_blob_purges
         WHERE storage_key = ? AND processed_at IS NULL
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )
    .bind(storage_key)
    .fetch_optional(pool)
    .await
}

/// Marks a purge obligation as completed.
pub async fn mark_asset_purge_processed(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE asset_blob_purges SET processed_at = ?, last_error = NULL WHERE id = ?")
        .bind(now_timestamp())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records a failed purge without clearing the pending state.
pub async fn mark_asset_purge_failed(
    pool: &SqlitePool,
    id: &str,
    attempt: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE asset_blob_purges SET attempts = ?, last_error = ? WHERE id = ?")
        .bind(attempt.max(0))
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
