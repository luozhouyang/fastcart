//! Generic persistent workflow engine.
//!
//! Workflows are ordered lists of named steps. Executions, step checkpoints,
//! retry counts and leases live in the store database so a crashed runner can
//! resume where it stopped and concurrent runners cannot both advance the same
//! execution.
//!
//! The engine deliberately never holds a SQLite transaction while awaiting
//! arbitrary step code. Each state transition is its own short transaction and
//! carries a fencing check against the runner's lease.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{AppError, PageResult, Pagination, now_timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// Persisted workflow execution statuses.
pub mod status {
    /// Waiting to be picked up by a runner.
    pub const PENDING: &str = "pending";
    /// A runner holds the lease.
    pub const RUNNING: &str = "running";
    /// Every step succeeded.
    pub const COMPLETED: &str = "completed";
    /// A step failed and compensation finished.
    pub const FAILED: &str = "failed";

    /// Persisted workflow step statuses.
    pub mod step {
        /// Not attempted yet.
        pub const PENDING: &str = "pending";
        /// An attempt is in flight.
        pub const RUNNING: &str = "running";
        /// The step returned successfully.
        pub const SUCCEEDED: &str = "succeeded";
        /// The step exhausted its attempts.
        pub const FAILED: &str = "failed";
        /// Compensation completed.
        pub const COMPENSATED: &str = "compensated";
        /// Compensation ran but returned an error.
        pub const COMPENSATION_FAILED: &str = "compensation_failed";
    }
}

/// Error returned by a workflow step.
///
/// `retryable` lets a step opt out of the engine retry budget, for example for
/// validation failures that cannot succeed on a second attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepError {
    /// Human-readable failure message.
    pub message: String,
    /// Whether the engine may retry this step.
    pub retryable: bool,
}

impl StepError {
    /// Creates a retryable step error.
    #[must_use]
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    /// Creates a non-retryable step error.
    #[must_use]
    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    /// Returns true when the engine may retry the step.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for StepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StepError {}

/// Context passed to a step implementation.
#[derive(Clone, Debug)]
pub struct StepContext {
    /// Execution identifier.
    pub execution_id: String,
    /// Name of the step being executed.
    pub step_name: String,
    /// Workflow input.
    pub input: Value,
    /// Outputs of steps that already succeeded, keyed by step name.
    pub outputs: Value,
    /// One-based attempt number for this step.
    pub attempt: i64,
}

impl StepContext {
    /// Returns the accumulated state, which is the map of step outputs.
    #[must_use]
    pub fn state(&self) -> &Value {
        &self.outputs
    }

    /// Returns the output of an earlier step, if present.
    #[must_use]
    pub fn output(&self, step_name: &str) -> Option<&Value> {
        self.outputs.get(step_name)
    }
}

/// One executable workflow step.
#[async_trait]
pub trait WorkflowStep: Send + Sync {
    /// Runs the step and returns its JSON output.
    async fn execute(&self, context: &StepContext) -> Result<Value, StepError>;

    /// Reverts a previously succeeded step.
    ///
    /// The default is a no-op so steps without side effects need no extra code.
    async fn compensate(&self, _context: &StepContext, _output: &Value) -> Result<(), StepError> {
        Ok(())
    }
}

/// Ordered definition of a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Stable workflow identifier. Idempotency keys are scoped to this value.
    pub id: String,
    /// Ordered step names.
    pub steps: Vec<String>,
}

impl WorkflowDefinition {
    /// Creates a definition from any iterable of step names.
    #[must_use]
    pub fn new<I, S>(id: impl Into<String>, steps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            id: id.into(),
            steps: steps.into_iter().map(Into::into).collect(),
        }
    }
}

/// Persisted workflow execution row.
#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct WorkflowExecution {
    /// Execution identifier.
    pub id: String,
    /// Workflow identifier.
    pub workflow_id: String,
    /// Optional deduplication key scoped to the workflow.
    pub idempotency_key: Option<String>,
    /// Execution status.
    pub status: String,
    /// Workflow input as stored JSON.
    pub input: String,
    /// Final step output as stored JSON.
    pub output: Option<String>,
    /// Last failure message.
    pub error: Option<String>,
    /// Total step attempts made by this execution.
    pub attempts: i64,
    /// Lease owner.
    pub locked_by: Option<String>,
    /// Lease expiry timestamp.
    pub locked_until: Option<i64>,
    /// First time a runner acquired the execution.
    pub started_at: Option<i64>,
    /// Completion or permanent-failure timestamp.
    pub finished_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl WorkflowExecution {
    /// Parses the stored workflow input.
    #[must_use]
    pub fn input_json(&self) -> Value {
        parse_json(&self.input)
    }

    /// Parses the stored execution output.
    #[must_use]
    pub fn output_json(&self) -> Option<Value> {
        self.output.as_deref().map(parse_json)
    }
}

/// Persisted workflow step checkpoint.
#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct WorkflowStepRecord {
    /// Step row identifier.
    pub id: String,
    /// Owning execution.
    pub execution_id: String,
    /// Zero-based position in the workflow.
    pub seq: i64,
    /// Step name.
    pub name: String,
    /// Step status.
    pub status: String,
    /// Step input as stored JSON.
    pub input: String,
    /// Step output as stored JSON.
    pub output: Option<String>,
    /// Last failure or compensation error.
    pub error: Option<String>,
    /// Attempts made against this step.
    pub attempts: i64,
    /// Compensation timestamp, whether it succeeded or failed.
    pub compensated_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl WorkflowStepRecord {
    /// Parses the stored step input.
    #[must_use]
    pub fn input_json(&self) -> Value {
        parse_json(&self.input)
    }

    /// Parses the stored step output.
    #[must_use]
    pub fn output_json(&self) -> Option<Value> {
        self.output.as_deref().map(parse_json)
    }
}

/// An execution together with its ordered step checkpoints.
#[derive(Clone, Debug)]
pub struct WorkflowExecutionDetail {
    /// Execution row.
    pub execution: WorkflowExecution,
    /// Step rows ordered by sequence.
    pub steps: Vec<WorkflowStepRecord>,
}

/// Optional execution list filters for admin inspection.
#[derive(Clone, Debug, Default)]
pub struct WorkflowExecutionFilter {
    /// Restrict to one workflow.
    pub workflow_id: Option<String>,
    /// Restrict to one status.
    pub status: Option<String>,
    /// Restrict to one idempotency key.
    pub idempotency_key: Option<String>,
}

/// Engine retry and lease policy.
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Maximum attempts per step, including the first one.
    pub max_attempts: i64,
    /// Lease length in seconds.
    pub lease_seconds: i64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            lease_seconds: 300,
        }
    }
}

/// Registers steps and runs persistent workflow executions.
#[derive(Clone)]
pub struct WorkflowEngine {
    database: StoreDatabase,
    steps: Arc<RwLock<HashMap<String, Arc<dyn WorkflowStep>>>>,
    config: EngineConfig,
}

impl WorkflowEngine {
    /// Creates an engine over one store database.
    #[must_use]
    pub fn new(database: StoreDatabase) -> Self {
        Self {
            database,
            steps: Arc::new(RwLock::new(HashMap::new())),
            config: EngineConfig::default(),
        }
    }

    /// Replaces the retry and lease policy.
    #[must_use]
    pub fn with_config(mut self, config: EngineConfig) -> Self {
        self.config = EngineConfig {
            max_attempts: config.max_attempts.max(1),
            lease_seconds: config.lease_seconds.max(0),
        };
        self
    }

    /// Sets the maximum attempts per step.
    #[must_use]
    pub fn with_max_attempts(mut self, max_attempts: i64) -> Self {
        self.config.max_attempts = max_attempts.max(1);
        self
    }

    /// Sets the lease length in seconds.
    #[must_use]
    pub fn with_lease_seconds(mut self, lease_seconds: i64) -> Self {
        self.config.lease_seconds = lease_seconds.max(0);
        self
    }

    /// Returns the underlying store database.
    #[must_use]
    pub fn database(&self) -> &StoreDatabase {
        &self.database
    }

    /// Registers a step implementation under a unique name.
    ///
    /// Registering the same name twice is rejected so a typo cannot silently
    /// replace the implementation used by persisted executions.
    pub fn register(
        &self,
        name: impl Into<String>,
        step: Arc<dyn WorkflowStep>,
    ) -> Result<(), AppError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(AppError::validation("workflow step name must not be empty"));
        }
        let mut steps = self
            .steps
            .write()
            .map_err(|_| AppError::internal("workflow step registry is poisoned"))?;
        if steps.contains_key(&name) {
            return Err(AppError::conflict(format!(
                "workflow step already registered: {name}"
            )));
        }
        steps.insert(name, step);
        Ok(())
    }

    /// Runs or resumes a workflow execution.
    ///
    /// With an idempotency key, a duplicate call returns the existing execution
    /// instead of creating a second one. A non-terminal execution is resumed
    /// once its lease is acquired; terminal executions are returned untouched.
    pub async fn run(
        &self,
        definition: &WorkflowDefinition,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<WorkflowExecution, AppError> {
        self.validate_definition(definition)?;
        let (execution, _) = self
            .get_or_create(definition, &input, idempotency_key.as_deref())
            .await?;
        if is_terminal(&execution.status) {
            return Ok(execution);
        }

        let owner = format!("wfrun_{}", Uuid::now_v7().simple());
        if !self.acquire_lease(&execution.id, &owner).await? {
            // Another runner holds the lease. Return its current snapshot.
            return self.get_execution_row(&execution.id).await;
        }
        self.drive(&execution.id, &owner).await
    }

    /// Returns one execution with its step checkpoints.
    pub async fn get(&self, id: &str) -> Result<WorkflowExecutionDetail, AppError> {
        let execution = self.get_execution_row(id).await?;
        let steps = self.load_steps(id).await?;
        Ok(WorkflowExecutionDetail { execution, steps })
    }

    /// Lists executions, with step checkpoints, newest first.
    pub async fn list(
        &self,
        filter: &WorkflowExecutionFilter,
        pagination: Pagination,
    ) -> Result<PageResult<WorkflowExecutionDetail>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let workflow_id = filter.workflow_id.as_deref();
        let status = filter.status.as_deref();
        let idempotency_key = filter.idempotency_key.as_deref();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM workflow_executions
             WHERE (?1 IS NULL OR workflow_id = ?1)
               AND (?2 IS NULL OR status = ?2)
               AND (?3 IS NULL OR idempotency_key = ?3)",
        )
        .bind(workflow_id)
        .bind(status)
        .bind(idempotency_key)
        .fetch_one(self.database.pool())
        .await
        .map_err(database_error)?;

        let executions = sqlx::query_as::<_, WorkflowExecution>(
            "SELECT * FROM workflow_executions
             WHERE (?1 IS NULL OR workflow_id = ?1)
               AND (?2 IS NULL OR status = ?2)
               AND (?3 IS NULL OR idempotency_key = ?3)
             ORDER BY created_at DESC, id DESC
             LIMIT ?4 OFFSET ?5",
        )
        .bind(workflow_id)
        .bind(status)
        .bind(idempotency_key)
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.database.pool())
        .await
        .map_err(database_error)?;

        let mut data = Vec::with_capacity(executions.len());
        for execution in executions {
            let steps = self.load_steps(&execution.id).await?;
            data.push(WorkflowExecutionDetail { execution, steps });
        }
        Ok(PageResult::new(data, count, pagination))
    }

    fn validate_definition(&self, definition: &WorkflowDefinition) -> Result<(), AppError> {
        if definition.id.trim().is_empty() {
            return Err(AppError::validation("workflow id must not be empty"));
        }
        let registry = self
            .steps
            .read()
            .map_err(|_| AppError::internal("workflow step registry is poisoned"))?;
        let mut seen = std::collections::HashSet::with_capacity(definition.steps.len());
        for name in &definition.steps {
            if name.trim().is_empty() {
                return Err(AppError::validation("workflow step name must not be empty"));
            }
            if !seen.insert(name.as_str()) {
                return Err(AppError::validation(format!(
                    "workflow definition contains duplicate step: {name}"
                )));
            }
            if !registry.contains_key(name) {
                return Err(AppError::not_found("workflow step", name));
            }
        }
        Ok(())
    }

    fn step(&self, name: &str) -> Option<Arc<dyn WorkflowStep>> {
        self.steps
            .read()
            .ok()
            .and_then(|steps| steps.get(name).cloned())
    }

    async fn get_or_create(
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

    async fn get_execution_row(&self, id: &str) -> Result<WorkflowExecution, AppError> {
        sqlx::query_as::<_, WorkflowExecution>("SELECT * FROM workflow_executions WHERE id = ?")
            .bind(id)
            .fetch_optional(self.database.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("workflow execution", id))
    }

    async fn load_steps(&self, execution_id: &str) -> Result<Vec<WorkflowStepRecord>, AppError> {
        sqlx::query_as::<_, WorkflowStepRecord>(
            "SELECT * FROM workflow_steps WHERE execution_id = ? ORDER BY seq ASC",
        )
        .bind(execution_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(database_error)
    }

    async fn acquire_lease(&self, execution_id: &str, owner: &str) -> Result<bool, AppError> {
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

    async fn renew_lease(&self, execution_id: &str, owner: &str) -> Result<bool, AppError> {
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
    async fn fence(
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

    async fn drive(&self, execution_id: &str, owner: &str) -> Result<WorkflowExecution, AppError> {
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

    async fn begin_attempt(
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
                     SET status = ?, error = NULL, compensated_at = ?, updated_at = ?
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
                     SET status = ?, error = ?, compensated_at = ?, updated_at = ?
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

    async fn compensate_and_fail(
        &self,
        execution: &WorkflowExecution,
        steps: &[WorkflowStepRecord],
        owner: &str,
    ) -> Result<WorkflowExecution, AppError> {
        for step in steps.iter().rev() {
            if step.status != status::step::SUCCEEDED {
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
        self.finalize_failure(&execution.id, owner).await
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

fn is_terminal(status: &str) -> bool {
    matches!(status, status::COMPLETED | status::FAILED)
}

fn is_step_done(status: &str) -> bool {
    matches!(status, status::step::SUCCEEDED | status::step::COMPENSATED)
}

fn parse_json(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or(Value::Null)
}

fn step_context(
    execution: &WorkflowExecution,
    steps: &[WorkflowStepRecord],
    step_name: &str,
    attempt: i64,
) -> StepContext {
    let mut outputs = serde_json::Map::new();
    for step in steps {
        if matches!(
            step.status.as_str(),
            status::step::SUCCEEDED | status::step::COMPENSATED | status::step::COMPENSATION_FAILED
        ) {
            if let Some(output) = &step.output {
                outputs.insert(step.name.clone(), parse_json(output));
            }
        }
    }
    StepContext {
        execution_id: execution.id.clone(),
        step_name: step_name.to_owned(),
        input: execution.input_json(),
        outputs: Value::Object(outputs),
        attempt,
    }
}
