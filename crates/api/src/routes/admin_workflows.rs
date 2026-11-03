//! Admin read-only workflow execution inspection.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::routing::get;
use fastcart_http::{AdminActor, ApiError, ApiResult, PaginationQuery};
use fastcart_kernel::ActorContext;
use fastcart_workflows::{
    WorkflowEngine, WorkflowExecutionDetail, WorkflowExecutionFilter, WorkflowStepRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::StoreDb;
use crate::dto::{PageInfo, PageResponse};
use crate::state::AppContext;

/// Builds workflow execution inspection routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/workflows-executions", get(list_executions))
        .route("/workflows-executions/{id}", get(get_execution))
}

/// A workflow step checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowStepDto {
    /// Step row identifier.
    pub id: String,
    /// Zero-based position in the workflow.
    pub seq: i64,
    /// Step name.
    pub name: String,
    /// Step status.
    pub status: String,
    /// Step input.
    pub input: Value,
    /// Step output, when the step succeeded.
    pub output: Option<Value>,
    /// Last failure or compensation error.
    pub error: Option<String>,
    /// Attempts made against this step.
    pub attempts: i64,
    /// Compensation timestamp.
    pub compensated_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<WorkflowStepRecord> for WorkflowStepDto {
    fn from(step: WorkflowStepRecord) -> Self {
        let input = step.input_json();
        let output = step.output_json();
        Self {
            id: step.id,
            seq: step.seq,
            name: step.name,
            status: step.status,
            input,
            output,
            error: step.error,
            attempts: step.attempts,
            compensated_at: step.compensated_at,
            created_at: step.created_at,
            updated_at: step.updated_at,
        }
    }
}

/// A workflow execution with its step checkpoints.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct WorkflowExecutionDto {
    /// Execution identifier.
    pub id: String,
    /// Workflow identifier.
    pub workflow_id: String,
    /// Idempotency key scoped to the workflow.
    pub idempotency_key: Option<String>,
    /// Execution status.
    pub status: String,
    /// Workflow input.
    pub input: Value,
    /// Final step output.
    pub output: Option<Value>,
    /// Last failure message.
    pub error: Option<String>,
    /// Total step attempts.
    pub attempts: i64,
    /// Current lease owner.
    pub locked_by: Option<String>,
    /// Lease expiry timestamp.
    pub locked_until: Option<i64>,
    /// First runner start timestamp.
    pub started_at: Option<i64>,
    /// Completion or permanent-failure timestamp.
    pub finished_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
    /// Ordered step checkpoints.
    pub steps: Vec<WorkflowStepDto>,
}

impl From<WorkflowExecutionDetail> for WorkflowExecutionDto {
    fn from(detail: WorkflowExecutionDetail) -> Self {
        let execution = detail.execution;
        let input = execution.input_json();
        let output = execution.output_json();
        Self {
            id: execution.id,
            workflow_id: execution.workflow_id,
            idempotency_key: execution.idempotency_key,
            status: execution.status,
            input,
            output,
            error: execution.error,
            attempts: execution.attempts,
            locked_by: execution.locked_by,
            locked_until: execution.locked_until,
            started_at: execution.started_at,
            finished_at: execution.finished_at,
            created_at: execution.created_at,
            updated_at: execution.updated_at,
            steps: detail
                .steps
                .into_iter()
                .map(WorkflowStepDto::from)
                .collect(),
        }
    }
}

fn authorize(actor: &ActorContext) -> ApiResult<()> {
    actor
        .require_permission("settings.read")
        .map_err(ApiError::new)
}

async fn list_executions(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<WorkflowExecutionDto>>> {
    authorize(&actor)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = WorkflowEngine::new(db)
        .list(&WorkflowExecutionFilter::default(), pagination)
        .await?;
    Ok(Json(PageResponse {
        data: page
            .data
            .into_iter()
            .map(WorkflowExecutionDto::from)
            .collect(),
        pagination: PageInfo {
            limit: page.pagination.limit,
            offset: page.pagination.offset,
            total: page.count,
        },
    }))
}

async fn get_execution(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<WorkflowExecutionDto>> {
    authorize(&actor)?;
    let detail = WorkflowEngine::new(db).get(&id).await?;
    Ok(Json(WorkflowExecutionDto::from(detail)))
}
