//! Admin AI tool catalog and agent-task lifecycle.
//!
//! Read-only tools execute immediately when a task is created. Mutating tools
//! are recorded as `pending` and run only after an administrator approves
//! them, so every state change keeps its human gate. A task is claimed under
//! `BEGIN IMMEDIATE` (`pending`/`awaiting_approval` -> `running`) and the tool
//! runs after the claim commits: no transaction is held during tool work, and
//! the guarded final update only matches the `running` row. Two concurrent
//! approvals therefore execute a task at most once.
//!
//! Every transition is written to the admin audit log, and the tool result or
//! structured error is stored on the task so the caller can read it back.

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_ai::execution::{
    BlockListTypesInput, OrderGetInput, PageGetInput, PagePublishInput, PageUpdateInput,
    ToolExecutionError, ToolExecutor, ToolRuntime, validate_tool_input,
};
use fastcart_ai::{AgentTaskStatus, ToolDefinition};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{ActorContext, AppError, OrderId, PageId, PageResult, now_timestamp};
use fastcart_module_content::{ContentService, PageRevisionOrigin, UpdatePageInput};
use fastcart_module_order::OrderService;
use fastcart_plugin::Contribution;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, record_store_audit};
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the admin AI routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/ai/tools", get(list_tools))
        .route("/ai/tasks", get(list_tasks).post(create_task))
        .route("/ai/tasks/{id}", get(get_task))
        .route("/ai/tasks/{id}/approve", post(approve_task))
        .route("/ai/tasks/{id}/reject", post(reject_task))
}

/// Builds the documented admin AI routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_tools))
        .routes(routes!(list_tasks, create_task))
        .routes(routes!(get_task))
        .routes(routes!(approve_task))
        .routes(routes!(reject_task))
}

fn require(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

/// One tool definition exposed to administrators.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct AiToolDto {
    /// Stable tool name.
    pub name: String,
    /// Description presented to the model.
    pub description: String,
    /// JSON Schema for tool input.
    #[schema(value_type = Object)]
    pub input_schema: Value,
    /// Whether the tool mutates state.
    pub mutates: bool,
    /// Whether a human must approve an invocation.
    pub requires_approval: bool,
}

impl From<ToolDefinition> for AiToolDto {
    fn from(tool: ToolDefinition) -> Self {
        Self {
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
            mutates: tool.mutates,
            requires_approval: tool.requires_approval,
        }
    }
}

/// A recorded agent task.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct AiTaskDto {
    /// Task identifier.
    pub id: String,
    /// Stable tool name the task invokes.
    pub tool: String,
    /// Recorded invocation input.
    #[schema(value_type = Object)]
    pub input: Value,
    /// Tool output, when execution succeeded.
    #[schema(value_type = Object)]
    pub output: Option<Value>,
    /// Lifecycle status.
    pub status: String,
    /// Structured error, when the task failed.
    pub error: Option<String>,
    /// Administrator that requested the task.
    pub created_by: Option<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

/// Task creation payload.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateAiTaskRequest {
    /// Stable tool name.
    pub tool: String,
    /// Invocation input; must be a JSON object.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub input: Value,
}

/// Row shape persisted in `ai_tasks`.
#[derive(Debug, sqlx::FromRow)]
struct AiTaskRow {
    id: String,
    tool: String,
    input_json: String,
    output_json: Option<String>,
    status: String,
    error: Option<String>,
    created_by: Option<String>,
    created_at: i64,
    updated_at: i64,
}

const TASK_COLUMNS: &str = "id, tool, input_json, output_json, status, error, created_by, \
     created_at, updated_at";

/// How long a runner may hold a claimed task before another approval takes it
/// over.
///
/// Execution is a synchronous request, so a task left `running` means the
/// runner died mid-flight. Without a lease such a task could never be retried.
/// The window is generous because every builtin tool is a short database
/// operation.
const EXECUTION_LEASE_SECONDS: i64 = 300;

fn task_dto(row: AiTaskRow) -> ApiResult<AiTaskDto> {
    let status = AgentTaskStatus::parse(&row.status).ok_or_else(|| {
        ApiError::new(AppError::internal(format!(
            "unknown stored agent task status: {}",
            row.status
        )))
    })?;
    let input = serde_json::from_str(&row.input_json).map_err(|error| {
        ApiError::new(AppError::internal(format!(
            "invalid stored agent task input: {error}"
        )))
    })?;
    let output = row
        .output_json
        .map(|raw| serde_json::from_str(&raw))
        .transpose()
        .map_err(|error| {
            ApiError::new(AppError::internal(format!(
                "invalid stored agent task output: {error}"
            )))
        })?;
    Ok(AiTaskDto {
        id: row.id,
        tool: row.tool,
        input,
        output,
        status: status.as_str().to_owned(),
        error: row.error,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn db_error(error: sqlx::Error) -> ApiError {
    ApiError::new(database_error(error))
}

async fn fetch_task_row(db: &StoreDatabase, id: &str) -> ApiResult<Option<AiTaskRow>> {
    sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM ai_tasks WHERE id = ?"))
        .bind(id)
        .fetch_optional(db.pool())
        .await
        .map_err(db_error)
}

#[utoipa::path(
    get,
    path = "/ai/tools",
    tag = "Admin",
    summary = "List AI tool definitions",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<AiToolDto>))
)]
async fn list_tools(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
) -> ApiResult<Json<Vec<AiToolDto>>> {
    require(&actor, "settings.read")?;
    Ok(Json(
        context
            .ai
            .tools()
            .into_iter()
            .map(AiToolDto::from)
            .collect(),
    ))
}

#[utoipa::path(
    get,
    path = "/ai/tasks",
    tag = "Admin",
    summary = "List AI agent tasks",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<AiTaskDto>))
)]
async fn list_tasks(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<AiTaskDto>>> {
    require(&actor, "settings.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ai_tasks")
        .fetch_one(db.pool())
        .await
        .map_err(db_error)?;
    let rows: Vec<AiTaskRow> = sqlx::query_as(&format!(
        "SELECT {TASK_COLUMNS} FROM ai_tasks ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
    ))
    .bind(pagination.limit)
    .bind(pagination.offset)
    .fetch_all(db.pool())
    .await
    .map_err(db_error)?;
    let data = rows
        .into_iter()
        .map(task_dto)
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(PageResponse::from_page(PageResult::new(
        data, total, pagination,
    ))))
}

#[utoipa::path(
    get,
    path = "/ai/tasks/{id}",
    tag = "Admin",
    summary = "Get an AI agent task",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = AiTaskDto))
)]
async fn get_task(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<AiTaskDto>> {
    require(&actor, "settings.read")?;
    let Some(row) = fetch_task_row(&db, &id).await? else {
        return Err(ApiError::not_found(format!("ai task not found: {id}")));
    };
    Ok(Json(task_dto(row)?))
}

#[utoipa::path(
    post,
    path = "/ai/tasks",
    tag = "Admin",
    summary = "Create an AI agent task",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateAiTaskRequest,
    responses((status = 201, body = AiTaskDto))
)]
async fn create_task(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateAiTaskRequest>,
) -> ApiResult<(StatusCode, Json<AiTaskDto>)> {
    require(&actor, "settings.write")?;
    let tool = context.ai.tool(&request.tool).ok_or_else(|| {
        ApiError::new(AppError::validation(format!(
            "unknown AI tool: {}",
            request.tool
        )))
    })?;
    let input = if request.input.is_null() {
        Value::Object(Default::default())
    } else {
        request.input
    };
    if !input.is_object() {
        return Err(ApiError::new(AppError::validation(
            "AI task input must be a JSON object",
        )));
    }
    validate_tool_input(&context.ai, &tool.name, &input)
        .map_err(|error| validation_error(error, &tool.name))?;

    let user_id = actor.require_user().map_err(ApiError::new)?;
    let id = format!("task_{}", uuid::Uuid::now_v7().simple());
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO ai_tasks (id, tool, input_json, status, error, created_by, created_at, \
         updated_at) VALUES (?, ?, ?, ?, NULL, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&tool.name)
    .bind(input.to_string())
    .bind(AgentTaskStatus::Pending.as_str())
    .bind(user_id.as_str())
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(db_error)?;

    record_store_audit(
        &context,
        &actor,
        "ai.task.created",
        "ai_task",
        &id,
        serde_json::json!({"tool": tool.name, "requires_approval": tool.requires_approval}),
    )
    .await;

    // Read-only tools close the loop immediately. Mutating tools stay pending
    // until an administrator approves them.
    if tool.requires_approval {
        let Some(row) = fetch_task_row(&db, &id).await? else {
            return Err(ApiError::new(AppError::internal(
                "AI task disappeared after creation",
            )));
        };
        return Ok((StatusCode::CREATED, Json(task_dto(row)?)));
    }

    let row = claim_task(&db, &id).await?;
    let task = execute_claimed(&context, &actor, &db, row).await?;
    Ok((StatusCode::CREATED, Json(task)))
}

#[utoipa::path(
    post,
    path = "/ai/tasks/{id}/approve",
    tag = "Admin",
    summary = "Approve and execute an AI agent task",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = AiTaskDto))
)]
async fn approve_task(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<AiTaskDto>> {
    require(&actor, "settings.write")?;
    let row = claim_task(&db, &id).await?;
    record_store_audit(
        &context,
        &actor,
        "ai.task.approved",
        "ai_task",
        &id,
        serde_json::json!({"tool": row.tool, "status": AgentTaskStatus::Running.as_str()}),
    )
    .await;
    let task = execute_claimed(&context, &actor, &db, row).await?;
    Ok(Json(task))
}

#[utoipa::path(
    post,
    path = "/ai/tasks/{id}/reject",
    tag = "Admin",
    summary = "Reject an AI agent task",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = AiTaskDto))
)]
async fn reject_task(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<AiTaskDto>> {
    require(&actor, "settings.write")?;
    let task = transition(&db, &id, AgentTaskStatus::Cancelled).await?;
    record_store_audit(
        &context,
        &actor,
        "ai.task.rejected",
        "ai_task",
        &id,
        serde_json::json!({"tool": task.tool, "status": task.status}),
    )
    .await;
    Ok(Json(task))
}

fn validation_error(error: ToolExecutionError, tool: &str) -> ApiError {
    match error {
        ToolExecutionError::UnknownTool(_) => {
            ApiError::new(AppError::validation(format!("unknown AI tool: {tool}")))
        }
        ToolExecutionError::InvalidInput(message) => ApiError::new(AppError::validation(message)),
        other => ApiError::new(AppError::internal(other.to_string())),
    }
}

/// Claims a task for execution inside an immediate write transaction.
///
/// Only `pending` and `awaiting_approval` are claimable. `running -> running`
/// is a legal state-machine transition, so the guard must name the source
/// states explicitly; otherwise a second approval that merely re-read a
/// running task could claim it again.
async fn claim_task(db: &StoreDatabase, id: &str) -> ApiResult<AiTaskRow> {
    let mut tx = db
        .begin()
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    let now = now_timestamp();
    let stale_before = now.saturating_sub(EXECUTION_LEASE_SECONDS);
    let updated = sqlx::query(
        "UPDATE ai_tasks SET status = ?, started_at = ?, updated_at = ? \
         WHERE id = ? AND (status IN (?, ?) OR (status = ? AND started_at IS NOT NULL \
         AND started_at < ?))",
    )
    .bind(AgentTaskStatus::Running.as_str())
    .bind(now)
    .bind(now)
    .bind(id)
    .bind(AgentTaskStatus::Pending.as_str())
    .bind(AgentTaskStatus::AwaitingApproval.as_str())
    .bind(AgentTaskStatus::Running.as_str())
    .bind(stale_before)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;

    if updated.rows_affected() == 0 {
        let row: Option<AiTaskRow> =
            sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM ai_tasks WHERE id = ?"))
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?;
        let Some(row) = row else {
            return Err(ApiError::not_found(format!("ai task not found: {id}")));
        };
        let current = AgentTaskStatus::parse(&row.status).ok_or_else(|| {
            ApiError::new(AppError::internal(format!(
                "unknown stored agent task status: {}",
                row.status
            )))
        })?;
        return Err(ApiError::new(AppError::conflict_details(
            "illegal AI agent task transition",
            serde_json::json!({
                "from": current.as_str(),
                "to": AgentTaskStatus::Running.as_str(),
            }),
        )));
    }

    let row: AiTaskRow =
        sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM ai_tasks WHERE id = ?"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(row)
}

/// Executes a claimed task and persists its terminal state.
async fn execute_claimed(
    context: &Arc<AppContext>,
    actor: &ActorContext,
    db: &StoreDatabase,
    row: AiTaskRow,
) -> ApiResult<AiTaskDto> {
    let input: Value = serde_json::from_str(&row.input_json).map_err(|error| {
        ApiError::new(AppError::internal(format!(
            "invalid stored agent task input: {error}"
        )))
    })?;
    let runtime = StoreToolRuntime {
        context: context.clone(),
        db: db.clone(),
        actor_id: actor.user_id.as_ref().map(|id| id.as_str().to_owned()),
    };
    let executor = ToolExecutor::new(&context.ai, &runtime);
    let outcome = executor.execute(&row.tool, &input).await;
    let task = finish_task(db, &row.id, outcome).await?;
    // A task cancelled while it was executing keeps `cancelled`; only the two
    // execution outcomes get an execution audit entry.
    let action = match task.status.as_str() {
        "succeeded" => Some("ai.task.succeeded"),
        "failed" => Some("ai.task.failed"),
        _ => None,
    };
    if let Some(action) = action {
        record_store_audit(
            context,
            actor,
            action,
            "ai_task",
            &row.id,
            serde_json::json!({
                "tool": &row.tool,
                "status": &task.status,
                "error": &task.error,
            }),
        )
        .await;
    }
    Ok(task)
}

/// Writes the terminal state of a claimed task.
///
/// The update is guarded by `status = 'running'`, so a task cancelled while it
/// was executing keeps its terminal `cancelled` state and the late result is
/// dropped instead of overwriting it.
async fn finish_task(
    db: &StoreDatabase,
    id: &str,
    outcome: Result<Value, ToolExecutionError>,
) -> ApiResult<AiTaskDto> {
    let now = now_timestamp();
    let (status, output_json, error) = match outcome {
        Ok(output) => {
            let output_json = serde_json::to_string(&output).map_err(|error| {
                ApiError::new(AppError::internal(format!(
                    "could not serialize AI tool output: {error}"
                )))
            })?;
            (AgentTaskStatus::Succeeded, Some(output_json), None)
        }
        Err(error) => (AgentTaskStatus::Failed, None, Some(error.to_stored_json())),
    };
    let updated = sqlx::query(
        "UPDATE ai_tasks SET status = ?, output_json = ?, error = ?, finished_at = ?, \
         updated_at = ? WHERE id = ? AND status = ?",
    )
    .bind(status.as_str())
    .bind(output_json)
    .bind(error)
    .bind(now)
    .bind(now)
    .bind(id)
    .bind(AgentTaskStatus::Running.as_str())
    .execute(db.pool())
    .await
    .map_err(db_error)?;
    if updated.rows_affected() == 0 {
        tracing::warn!(
            task_id = id,
            "AI task is no longer running; its execution result was not stored"
        );
    }
    let Some(row) = fetch_task_row(db, id).await? else {
        return Err(ApiError::not_found(format!("ai task not found: {id}")));
    };
    task_dto(row)
}

/// Applies one legal status transition inside an immediate transaction.
///
/// The write lock is taken before the row is read, so two concurrent
/// transitions cannot both observe the same status. Illegal transitions are
/// reported as conflicts, which the API renders as `409`.
async fn transition(db: &StoreDatabase, id: &str, next: AgentTaskStatus) -> ApiResult<AiTaskDto> {
    let mut tx = db
        .begin()
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    let row: Option<AiTaskRow> =
        sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM ai_tasks WHERE id = ?"))
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_error)?;
    let Some(row) = row else {
        return Err(ApiError::not_found(format!("ai task not found: {id}")));
    };
    let current = AgentTaskStatus::parse(&row.status).ok_or_else(|| {
        ApiError::new(AppError::internal(format!(
            "unknown stored agent task status: {}",
            row.status
        )))
    })?;
    if !current.can_transition_to(next) {
        return Err(ApiError::new(AppError::conflict_details(
            "illegal AI agent task transition",
            serde_json::json!({"from": current.as_str(), "to": next.as_str()}),
        )));
    }

    let now = now_timestamp();
    sqlx::query("UPDATE ai_tasks SET status = ?, updated_at = ? WHERE id = ?")
        .bind(next.as_str())
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let row: AiTaskRow =
        sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM ai_tasks WHERE id = ?"))
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    task_dto(row)
}

/// Application-service runtime for the built-in tools.
///
/// Every operation goes through an existing service. The runtime never touches
/// SQLite directly, and mutations carry the acting administrator so the
/// service-level audit entry stays atomic with the change.
struct StoreToolRuntime {
    context: Arc<AppContext>,
    db: StoreDatabase,
    actor_id: Option<String>,
}

impl StoreToolRuntime {
    fn content(&self) -> ContentService {
        ContentService::new(self.db.clone()).with_audit_actor(self.actor_id.clone())
    }
}

fn service_error(error: AppError) -> ToolExecutionError {
    match error {
        AppError::NotFound(message) => ToolExecutionError::NotFound(message),
        AppError::Validation(message) => ToolExecutionError::InvalidInput(message),
        other => ToolExecutionError::Failed(other.to_string()),
    }
}

fn tool_json<T: Serialize>(value: T) -> Result<Value, ToolExecutionError> {
    serde_json::to_value(value).map_err(|error| {
        ToolExecutionError::Failed(format!("could not serialize tool output: {error}"))
    })
}

#[async_trait]
impl ToolRuntime for StoreToolRuntime {
    async fn get_page(&self, input: PageGetInput) -> Result<Value, ToolExecutionError> {
        let page = self
            .content()
            .get_page(&PageId::from_string(input.id))
            .await
            .map_err(service_error)?;
        tool_json(page)
    }

    async fn update_page(&self, input: PageUpdateInput) -> Result<Value, ToolExecutionError> {
        let page = self
            .content()
            .update_page(
                &PageId::from_string(input.id),
                UpdatePageInput {
                    title: input.title,
                    handle: input.handle,
                    page_type: input.page_type,
                    body: input.body,
                    seo: input.seo,
                    metadata: None,
                    base_revision: input.base_revision,
                    origin: Some(PageRevisionOrigin::Ai),
                    created_by: self.actor_id.clone(),
                },
            )
            .await
            .map_err(service_error)?;
        tool_json(page)
    }

    async fn publish_page(&self, input: PagePublishInput) -> Result<Value, ToolExecutionError> {
        let page = self
            .content()
            .publish_page_as(
                &PageId::from_string(input.id),
                self.actor_id.as_deref(),
                PageRevisionOrigin::Ai,
            )
            .await
            .map_err(service_error)?;
        tool_json(page)
    }

    async fn list_block_types(
        &self,
        _input: BlockListTypesInput,
    ) -> Result<Value, ToolExecutionError> {
        // Themes are deployment artifacts (ADR 0008): the active theme registry
        // stores identity and settings only, so it carries no editor manifest
        // and contributes no block keys. Plugin components come from the same
        // cached catalog the admin component route reads, and use the
        // `plugin:<plugin_id>:<component_id>` type the page editor stores in
        // page bodies.
        let catalog = super::plugin_catalog::load(&self.context, &self.db)
            .await
            .map_err(service_error)?;
        let registry = catalog.registry();
        let mut types = Vec::new();
        for provider in registry.components() {
            let Contribution::StorefrontComponent(component) = &provider.contribution else {
                continue;
            };
            types.push(format!("plugin:{}:{}", provider.plugin_id, component.id));
        }
        types.sort();
        types.dedup();
        Ok(Value::Array(types.into_iter().map(Value::String).collect()))
    }

    async fn get_order(&self, input: OrderGetInput) -> Result<Value, ToolExecutionError> {
        let order = OrderService::new(self.db.clone())
            .get(&OrderId::from_string(input.id))
            .await
            .map_err(service_error)?;
        tool_json(order)
    }
}
