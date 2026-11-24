//! Execution creation and checkpoint reads.

use super::*;

impl WorkflowEngine {
    pub(super) async fn get_or_create(
        &self,
        definition: &WorkflowDefinition,
        input: &Value,
        idempotency_key: Option<&str>,
    ) -> Result<(WorkflowExecution, bool), AppError> {
        let now = now_timestamp();
        let execution_id = format!("wfexec_{}", Uuid::now_v7().simple());
        let input_json = input.to_string();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;

        let inserted = sqlx::query(
            "INSERT INTO workflow_executions
             (id, workflow_id, idempotency_key, status, input, output, error,
              attempts, locked_by, locked_until, started_at, finished_at,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, NULL, NULL, 0, NULL, NULL, NULL, NULL, ?, ?)
             ON CONFLICT DO NOTHING",
        )
        .bind(&execution_id)
        .bind(&definition.id)
        .bind(idempotency_key)
        .bind(status::PENDING)
        .bind(&input_json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;

        if inserted.rows_affected() == 1 {
            for (seq, name) in definition.steps.iter().enumerate() {
                let step_id = format!("wfstep_{}", Uuid::now_v7().simple());
                let seq = i64::try_from(seq).unwrap_or(i64::MAX);
                sqlx::query(
                    "INSERT INTO workflow_steps
                     (id, execution_id, seq, name, status, input, output, error,
                      attempts, compensated_at, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, 0, NULL, ?, ?)",
                )
                .bind(step_id)
                .bind(&execution_id)
                .bind(seq)
                .bind(name)
                .bind(status::step::PENDING)
                .bind(&input_json)
                .bind(now)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            }
            tx.commit().await.map_err(database_error)?;
            let execution = self.get_execution_row(&execution_id).await?;
            return Ok((execution, true));
        }

        let Some(idempotency_key) = idempotency_key else {
            return Err(AppError::internal(
                "workflow execution insert conflicted without an idempotency key",
            ));
        };
        let execution = sqlx::query_as::<_, WorkflowExecution>(
            "SELECT * FROM workflow_executions
             WHERE workflow_id = ? AND idempotency_key = ?",
        )
        .bind(&definition.id)
        .bind(idempotency_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok((execution, false))
    }

    pub(super) async fn get_execution_row(&self, id: &str) -> Result<WorkflowExecution, AppError> {
        sqlx::query_as::<_, WorkflowExecution>("SELECT * FROM workflow_executions WHERE id = ?")
            .bind(id)
            .fetch_optional(self.database.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("workflow execution", id))
    }

    pub(super) async fn load_steps(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowStepRecord>, AppError> {
        sqlx::query_as::<_, WorkflowStepRecord>(
            "SELECT * FROM workflow_steps WHERE execution_id = ? ORDER BY seq ASC",
        )
        .bind(execution_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(database_error)
    }

    pub(super) async fn begin_attempt(
        &self,
        execution: &WorkflowExecution,
        step: &WorkflowStepRecord,
        owner: &str,
    ) -> Result<Option<i64>, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, &execution.id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return Ok(None);
        }

        let attempt = step.attempts.saturating_add(1);
        sqlx::query(
            "UPDATE workflow_steps
             SET status = ?, attempts = ?, error = NULL, updated_at = ?
             WHERE id = ? AND execution_id = ?",
        )
        .bind(status::step::RUNNING)
        .bind(attempt)
        .bind(now)
        .bind(&step.id)
        .bind(&execution.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query(
            "UPDATE workflow_executions SET attempts = attempts + 1, updated_at = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(&execution.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(Some(attempt))
    }
}
