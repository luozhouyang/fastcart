//! Driving an execution through its steps and compensation.

use super::*;

impl WorkflowEngine {
    pub(super) async fn drive(
        &self,
        execution_id: &str,
        owner: &str,
    ) -> Result<WorkflowExecution, AppError> {
        loop {
            if !self.renew_lease(execution_id, owner).await? {
                return self.get_execution_row(execution_id).await;
            }
            let execution = self.get_execution_row(execution_id).await?;
            if is_terminal(&execution.status) {
                return Ok(execution);
            }
            let steps = self.load_steps(execution_id).await?;

            if steps.iter().any(|step| {
                matches!(
                    step.status.as_str(),
                    status::step::FAILED | status::step::COMPENSATION_FAILED
                )
            }) {
                return self.compensate_and_fail(&execution, &steps, owner).await;
            }

            let Some(step) = steps.iter().find(|step| !is_step_done(&step.status)) else {
                return self.complete_execution(&execution, &steps, owner).await;
            };

            if step.attempts >= self.config.max_attempts {
                let error = StepError::non_retryable(format!(
                    "workflow step exhausted {} attempts: {}",
                    self.config.max_attempts, step.name
                ));
                if !self
                    .persist_step_failure(execution_id, &step.id, &error, owner)
                    .await?
                {
                    return self.get_execution_row(execution_id).await;
                }
                let execution = self.get_execution_row(execution_id).await?;
                let steps = self.load_steps(execution_id).await?;
                return self.compensate_and_fail(&execution, &steps, owner).await;
            }

            let Some(implementation) = self.step(&step.name) else {
                let error = StepError::non_retryable(format!(
                    "workflow step is not registered: {}",
                    step.name
                ));
                if !self
                    .persist_step_failure(execution_id, &step.id, &error, owner)
                    .await?
                {
                    return self.get_execution_row(execution_id).await;
                }
                let execution = self.get_execution_row(execution_id).await?;
                let steps = self.load_steps(execution_id).await?;
                return self.compensate_and_fail(&execution, &steps, owner).await;
            };

            let Some(attempt) = self.begin_attempt(&execution, step, owner).await? else {
                return self.get_execution_row(execution_id).await;
            };
            let context = step_context(&execution, &steps, &step.name, attempt);
            match implementation.execute(&context).await {
                Ok(output) => {
                    if !self
                        .persist_step_success(execution_id, &step.id, &output, owner)
                        .await?
                    {
                        return self.get_execution_row(execution_id).await;
                    }
                }
                Err(error) => {
                    let retry = error.is_retryable() && attempt < self.config.max_attempts;
                    if retry {
                        if !self
                            .persist_step_retry(execution_id, &step.id, &error, owner)
                            .await?
                        {
                            return self.get_execution_row(execution_id).await;
                        }
                    } else {
                        if !self
                            .persist_step_failure(execution_id, &step.id, &error, owner)
                            .await?
                        {
                            return self.get_execution_row(execution_id).await;
                        }
                        let execution = self.get_execution_row(execution_id).await?;
                        let steps = self.load_steps(execution_id).await?;
                        return self.compensate_and_fail(&execution, &steps, owner).await;
                    }
                }
            }
        }
    }

    async fn persist_step_success(
        &self,
        execution_id: &str,
        step_id: &str,
        output: &Value,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, execution_id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE workflow_steps
             SET status = ?, output = ?, error = NULL, compensated_at = NULL,
                 updated_at = ?
             WHERE id = ? AND execution_id = ?",
        )
        .bind(status::step::SUCCEEDED)
        .bind(output.to_string())
        .bind(now)
        .bind(step_id)
        .bind(execution_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn persist_step_retry(
        &self,
        execution_id: &str,
        step_id: &str,
        error: &StepError,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, execution_id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE workflow_steps
             SET status = ?, error = ?, updated_at = ?
             WHERE id = ? AND execution_id = ?",
        )
        .bind(status::step::PENDING)
        .bind(&error.message)
        .bind(now)
        .bind(step_id)
        .bind(execution_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn persist_step_failure(
        &self,
        execution_id: &str,
        step_id: &str,
        error: &StepError,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, execution_id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return Ok(false);
        }
        sqlx::query(
            "UPDATE workflow_steps
             SET status = ?, error = ?, updated_at = ?
             WHERE id = ? AND execution_id = ?",
        )
        .bind(status::step::FAILED)
        .bind(&error.message)
        .bind(now)
        .bind(step_id)
        .bind(execution_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        sqlx::query("UPDATE workflow_executions SET error = ?, updated_at = ? WHERE id = ?")
            .bind(&error.message)
            .bind(now)
            .bind(execution_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn persist_compensation(
        &self,
        execution_id: &str,
        step_id: &str,
        result: &Result<(), StepError>,
        owner: &str,
    ) -> Result<bool, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, execution_id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return Ok(false);
        }
        match result {
            Ok(()) => {
                sqlx::query(
                    "UPDATE workflow_steps
                     SET status = ?, error = NULL, compensated_at = ?,
                         compensation_attempts = compensation_attempts + 1, updated_at = ?
                     WHERE id = ? AND execution_id = ?",
                )
                .bind(status::step::COMPENSATED)
                .bind(now)
                .bind(now)
                .bind(step_id)
                .bind(execution_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE workflow_steps
                     SET status = ?, error = ?, compensated_at = ?,
                         compensation_attempts = compensation_attempts + 1, updated_at = ?
                     WHERE id = ? AND execution_id = ?",
                )
                .bind(status::step::COMPENSATION_FAILED)
                .bind(&error.message)
                .bind(now)
                .bind(now)
                .bind(step_id)
                .bind(execution_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            }
        }
        tx.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn complete_execution(
        &self,
        execution: &WorkflowExecution,
        steps: &[WorkflowStepRecord],
        owner: &str,
    ) -> Result<WorkflowExecution, AppError> {
        let now = now_timestamp();
        let output = steps
            .iter()
            .rev()
            .find(|step| {
                matches!(
                    step.status.as_str(),
                    status::step::SUCCEEDED | status::step::COMPENSATED
                )
            })
            .and_then(|step| step.output.as_deref())
            .map(str::to_owned);

        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, &execution.id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return self.get_execution_row(&execution.id).await;
        }
        sqlx::query(
            "UPDATE workflow_executions
             SET status = ?, output = ?, error = NULL, locked_by = NULL,
                 locked_until = NULL, finished_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status::COMPLETED)
        .bind(output)
        .bind(now)
        .bind(now)
        .bind(&execution.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.get_execution_row(&execution.id).await
    }

    /// Compensates every succeeded step in reverse order, then finalizes the
    /// execution as failed.
    ///
    /// Compensation failures are not terminal on their own: a step left in
    /// `compensation_failed` is retried by later drives until it succeeds or
    /// exhausts `max_attempts` compensation attempts. While retries remain the
    /// execution is released back to `pending` instead of becoming `failed`,
    /// so a caller can resume it with the same idempotency key. Successful
    /// compensations are checkpointed and never re-run.
    async fn compensate_and_fail(
        &self,
        execution: &WorkflowExecution,
        steps: &[WorkflowStepRecord],
        owner: &str,
    ) -> Result<WorkflowExecution, AppError> {
        for step in steps.iter().rev() {
            let compensable = matches!(
                step.status.as_str(),
                status::step::SUCCEEDED | status::step::COMPENSATION_FAILED
            );
            if !compensable {
                continue;
            }
            if step.status == status::step::COMPENSATION_FAILED
                && step.compensation_attempts >= self.config.max_attempts
            {
                continue;
            }
            if !self.renew_lease(&execution.id, owner).await? {
                return self.get_execution_row(&execution.id).await;
            }

            let output = step
                .output
                .as_deref()
                .map(parse_json)
                .unwrap_or(Value::Null);
            let context = step_context(execution, steps, &step.name, step.attempts);
            let result = match self.step(&step.name) {
                Some(implementation) => implementation.compensate(&context, &output).await,
                None => Err(StepError::non_retryable(format!(
                    "workflow step is not registered: {}",
                    step.name
                ))),
            };
            if !self
                .persist_compensation(&execution.id, &step.id, &result, owner)
                .await?
            {
                return self.get_execution_row(&execution.id).await;
            }
        }

        let steps = self.load_steps(&execution.id).await?;
        let retry_pending = steps.iter().any(|step| {
            step.status == status::step::COMPENSATION_FAILED
                && step.compensation_attempts < self.config.max_attempts
        });
        if retry_pending {
            // Leave the execution resumable rather than terminal so another
            // drive can retry the remaining compensations.
            if !self.release_for_retry(&execution.id, owner).await? {
                return self.get_execution_row(&execution.id).await;
            }
            return self.get_execution_row(&execution.id).await;
        }
        self.finalize_failure(&execution.id, owner).await
    }

    /// Releases the lease and parks a non-terminal execution for a later drive.
    async fn release_for_retry(&self, execution_id: &str, owner: &str) -> Result<bool, AppError> {
        let now = now_timestamp();
        let result = sqlx::query(
            "UPDATE workflow_executions
             SET status = ?, locked_by = NULL, locked_until = NULL, updated_at = ?
             WHERE id = ? AND locked_by = ? AND status = ?",
        )
        .bind(status::PENDING)
        .bind(now)
        .bind(execution_id)
        .bind(owner)
        .bind(status::RUNNING)
        .execute(self.database.pool())
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() > 0)
    }

    async fn finalize_failure(
        &self,
        execution_id: &str,
        owner: &str,
    ) -> Result<WorkflowExecution, AppError> {
        let now = now_timestamp();
        let mut tx = self.database.begin().await.map_err(AppError::from)?;
        if !self.fence(&mut tx, execution_id, owner).await? {
            tx.rollback().await.map_err(database_error)?;
            return self.get_execution_row(execution_id).await;
        }
        sqlx::query(
            "UPDATE workflow_executions
             SET status = ?, locked_by = NULL, locked_until = NULL,
                 finished_at = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(status::FAILED)
        .bind(now)
        .bind(now)
        .bind(execution_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.get_execution_row(execution_id).await
    }
}
