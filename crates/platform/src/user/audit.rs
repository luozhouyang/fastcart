//! Platform audit records.

use super::*;

/// A new audit record.
#[derive(Clone, Debug)]
pub struct NewAudit {
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
    /// Request IP address.
    pub ip_address: Option<String>,
}

impl NewAudit {
    /// Creates an audit entry for an action.
    #[must_use]
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            actor_id: None,
            store_id: None,
            action: action.into(),
            resource_type: None,
            resource_id: None,
            data: serde_json::Value::Null,
            ip_address: None,
        }
    }
}

/// An audit record.
#[derive(Clone, Debug, FromRow, serde::Serialize, serde::Deserialize)]
pub struct AuditLog {
    /// Record identifier.
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
    /// Request IP address.
    pub ip_address: Option<String>,
    /// Timestamp.
    pub created_at: i64,
}

impl PlatformStore {
    /// Appends an audit record with a generated identifier.
    pub async fn record_audit(&self, entry: NewAudit) -> PlatformResult<()> {
        let id = format!("audit_{}", uuid::Uuid::now_v7().simple());
        self.record_audit_with_id(&id, &entry).await
    }
}

impl PlatformStore {
    /// Appends an audit record under a caller-chosen identifier.
    ///
    /// Re-delivering the same identifier is ignored, which makes outbox
    /// retries idempotent when delivery succeeds but the outbox row is not
    /// marked processed.
    pub async fn record_audit_with_id(&self, id: &str, entry: &NewAudit) -> PlatformResult<()> {
        sqlx::query(AUDIT_INSERT)
            .bind(id)
            .bind(entry.actor_id.as_deref())
            .bind(entry.store_id.as_deref())
            .bind(&entry.action)
            .bind(entry.resource_type.as_deref())
            .bind(entry.resource_id.as_deref())
            .bind(entry.data.to_string())
            .bind(entry.ip_address.as_deref())
            .bind(now_timestamp())
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

impl PlatformStore {
    /// Appends an audit record inside a mutation transaction.
    ///
    /// Uses the same insert-or-ignore semantics as [`Self::record_audit_with_id`]
    /// so a mutation and its audit row commit or roll back together.
    pub(crate) async fn append_audit_tx(
        transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: &str,
        entry: &NewAudit,
    ) -> PlatformResult<()> {
        sqlx::query(AUDIT_INSERT)
            .bind(id)
            .bind(entry.actor_id.as_deref())
            .bind(entry.store_id.as_deref())
            .bind(&entry.action)
            .bind(entry.resource_type.as_deref())
            .bind(entry.resource_id.as_deref())
            .bind(entry.data.to_string())
            .bind(entry.ip_address.as_deref())
            .bind(now_timestamp())
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }
}

impl PlatformStore {
    /// Lists recent audit records for a store.
    pub async fn list_audit_logs(
        &self,
        store_id: Option<&StoreId>,
        pagination: Pagination,
    ) -> PlatformResult<Vec<AuditLog>> {
        let rows =
            match store_id {
                Some(store_id) => {
                    sqlx::query_as::<_, AuditLog>(
                        "SELECT * FROM platform_audit_logs WHERE store_id = ?
                     ORDER BY created_at DESC LIMIT ? OFFSET ?",
                    )
                    .bind(store_id.as_str())
                    .bind(pagination.limit)
                    .bind(pagination.offset)
                    .fetch_all(self.pool())
                    .await?
                }
                None => sqlx::query_as::<_, AuditLog>(
                    "SELECT * FROM platform_audit_logs ORDER BY created_at DESC LIMIT ? OFFSET ?",
                )
                .bind(pagination.limit)
                .bind(pagination.offset)
                .fetch_all(self.pool())
                .await?,
            };
        Ok(rows)
    }
}
