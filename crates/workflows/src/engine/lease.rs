//! Execution lease acquisition, renewal and fencing.

use super::*;

impl WorkflowEngine {
    pub(super) async fn acquire_lease(
        &self,
        execution_id: &str,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE workflow_executions
             SET status = ?, locked_by = ?, locked_until = ?,
                 started_at = COALESCE(started_at, ?), updated_at = ?
             WHERE id = ?
               AND status IN (?, ?)
               AND (locked_until IS NULL OR locked_until < ?)",
        )
        .bind(status::RUNNING)
        .bind(owner)
        .bind(now.saturating_add(self.config.lease_seconds))
        .bind(now)
        .bind(now)
        .bind(execution_id)
        .bind(status::PENDING)
        .bind(status::RUNNING)
        .bind(now)
        .execute(self.database.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }

    pub(super) async fn renew_lease(
        &self,
        execution_id: &str,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE workflow_executions
             SET locked_until = ?, updated_at = ?
             WHERE id = ? AND locked_by = ? AND locked_until >= ?",
        )
        .bind(now.saturating_add(self.config.lease_seconds))
        .bind(now)
        .bind(execution_id)
        .bind(owner)
        .bind(now)
        .execute(self.database.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }

    /// Renews the lease inside a transaction so no other runner can advance the
    /// execution between the fence check and the state write.
    pub(super) async fn fence(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        execution_id: &str,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE workflow_executions
             SET locked_until = ?, updated_at = ?
             WHERE id = ? AND locked_by = ? AND locked_until >= ?",
        )
        .bind(now.saturating_add(self.config.lease_seconds))
        .bind(now)
        .bind(execution_id)
        .bind(owner)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }
}
