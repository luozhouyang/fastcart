//! Durable store-level audit outbox.
//!
//! Admin mutations append an audit row to the store database before delivery.
//! A successful immediate write to the platform audit log marks the row
//! processed; otherwise maintenance retries it from the store database.

use fastcart_kernel::now_timestamp;
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};

/// A new store audit row awaiting delivery.
///
/// Mirrors the platform `NewAudit` shape without coupling
/// infrastructure to the platform crate. `data` is serialized on insert.
#[derive(Clone, Debug)]
pub struct NewAuditOutbox {
    /// Acting user.
    pub actor_id: Option<String>,
    /// Store scope.
    pub store_id: Option<String>,
    /// Action name.
    pub action: String,
    /// Resource type.
    pub resource_type: Option<String>,
    /// Resource identifier.
    pub resource_id: Option<String>,
    /// JSON payload.
    pub data: serde_json::Value,
}

/// A persisted audit outbox row.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct AuditOutboxRecord {
    /// Audit identifier.
    pub id: String,
    /// Acting user.
    pub actor_id: Option<String>,
    /// Store scope.
    pub store_id: Option<String>,
    /// Action name.
    pub action: String,
    /// Resource type.
    pub resource_type: Option<String>,
    /// Resource identifier.
    pub resource_id: Option<String>,
    /// JSON payload.
    pub data: String,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Delivery attempts.
    pub attempts: i64,
    /// Last delivery error.
    pub last_error: Option<String>,
    /// Delivery timestamp, null while pending.
    pub processed_at: Option<i64>,
}

/// Appends an audit row using the caller's transaction or pool.
///
/// Passing a transaction guarantees the audit row commits atomically with the
/// business change it describes. Returns the generated identifier so callers
/// can mark immediate delivery without reading the row back.
pub async fn append_audit_outbox<'e, E>(
    executor: E,
    entry: &NewAuditOutbox,
) -> Result<String, sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    let id = format!("audit_{}", uuid::Uuid::now_v7().simple());
    let data = serde_json::to_string(&entry.data).unwrap_or_else(|_| "null".to_owned());
    sqlx::query(
        "INSERT INTO audit_outbox
         (id, actor_id, store_id, action, resource_type, resource_id, data, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(entry.actor_id.as_deref())
    .bind(entry.store_id.as_deref())
    .bind(&entry.action)
    .bind(entry.resource_type.as_deref())
    .bind(entry.resource_id.as_deref())
    .bind(data)
    .bind(now_timestamp())
    .execute(executor)
    .await?;
    Ok(id)
}

/// Fetches audit rows that have not yet reached the platform audit log.
pub async fn fetch_pending_audits(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<AuditOutboxRecord>, sqlx::Error> {
    sqlx::query_as::<_, AuditOutboxRecord>(
        "SELECT id, actor_id, store_id, action, resource_type, resource_id, data,
                created_at, attempts, last_error, processed_at
         FROM audit_outbox
         WHERE processed_at IS NULL
         ORDER BY created_at ASC
         LIMIT ?",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
}

/// Marks an audit row as delivered.
pub async fn mark_audit_processed(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE audit_outbox SET processed_at = ?, last_error = NULL WHERE id = ?")
        .bind(now_timestamp())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records a failed delivery without clearing the pending state.
pub async fn mark_audit_failed(
    pool: &SqlitePool,
    id: &str,
    attempt: i64,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE audit_outbox SET attempts = ?, last_error = ? WHERE id = ?")
        .bind(attempt.max(0))
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        NewAuditOutbox, append_audit_outbox, fetch_pending_audits, mark_audit_failed,
        mark_audit_processed,
    };
    use crate::database::connect_memory;

    async fn pool_with_schema() -> sqlx::SqlitePool {
        let pool = connect_memory().await.unwrap();
        sqlx::query(
            "CREATE TABLE audit_outbox (
                id TEXT PRIMARY KEY, actor_id TEXT, store_id TEXT,
                action TEXT NOT NULL, resource_type TEXT, resource_id TEXT,
                data TEXT NOT NULL, created_at INTEGER NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT,
                processed_at INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn entry() -> NewAuditOutbox {
        NewAuditOutbox {
            actor_id: Some("user_1".to_owned()),
            store_id: Some("store_1".to_owned()),
            action: "product.updated".to_owned(),
            resource_type: Some("product".to_owned()),
            resource_id: Some("prod_1".to_owned()),
            data: json!({"title": "sensitive"}),
        }
    }

    #[tokio::test]
    async fn audit_outbox_round_trip_marks_processed() {
        let pool = pool_with_schema().await;
        let id = append_audit_outbox(&pool, &entry()).await.unwrap();

        let pending = fetch_pending_audits(&pool, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].action, "product.updated");
        assert_eq!(pending[0].data, "{\"title\":\"sensitive\"}");

        mark_audit_processed(&pool, &id).await.unwrap();
        assert!(fetch_pending_audits(&pool, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_audit_stays_pending_with_attempt_count() {
        let pool = pool_with_schema().await;
        let id = append_audit_outbox(&pool, &entry()).await.unwrap();

        mark_audit_failed(&pool, &id, 1, "platform unavailable")
            .await
            .unwrap();

        let pending = fetch_pending_audits(&pool, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 1);
        assert_eq!(
            pending[0].last_error.as_deref(),
            Some("platform unavailable")
        );
    }
}
