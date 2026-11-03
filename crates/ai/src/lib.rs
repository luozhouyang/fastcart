//! AI extension foundation.
//!
//! AI tools invoke commerce application services only. They never access
//! repositories or SQLite directly, and mutations remain auditable. Tool
//! definitions are metadata: this crate never executes model or tool code.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use fastcart_kernel::{StoreId, UserId, now_timestamp};
use serde::{Deserialize, Serialize};

/// A controlled operation exposed to agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Stable tool name.
    pub name: String,
    /// Description presented to the model.
    pub description: String,
    /// JSON Schema for tool input. Must be an object schema.
    pub input_schema: serde_json::Value,
    /// Whether the tool mutates state.
    pub mutates: bool,
    /// Whether a human must approve an invocation.
    pub requires_approval: bool,
}

/// A model exposed by a provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelDefinition {
    /// Stable model identifier.
    pub id: String,
    /// Provider identifier.
    pub provider_id: String,
    /// Human-readable model name.
    pub name: String,
    /// Maximum context window in tokens.
    pub context_window: u32,
    /// Whether the model supports tool calling.
    pub supports_tools: bool,
}

/// An AI provider registration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderDefinition {
    /// Stable provider identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether the provider has credentials configured by the host.
    pub configured: bool,
}

/// Service errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AiError {
    /// A registry entry is invalid.
    #[error("invalid definition: {0}")]
    Invalid(String),
    /// A definition with the same id already exists.
    #[error("already registered: {0}")]
    Duplicate(String),
    /// A definition does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// A model refers to an unknown provider.
    #[error("unknown provider: {0}")]
    UnknownProvider(String),
    /// A task transition is not allowed.
    #[error("invalid task transition from {from:?} to {to:?}")]
    InvalidTransition {
        /// Current status.
        from: AgentTaskStatus,
        /// Requested status.
        to: AgentTaskStatus,
    },
    /// The task is missing required approval.
    #[error("task requires approval")]
    ApprovalRequired,
    /// A tool name is not registered.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
}

/// Lifecycle state of an agent task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    /// Waiting to run.
    Pending,
    /// Currently executing.
    Running,
    /// Waiting for human approval.
    AwaitingApproval,
    /// Completed successfully.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Cancelled by an actor.
    Cancelled,
}

impl AgentTaskStatus {
    /// Whether the status is terminal.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// Validates a task transition.
    #[must_use]
    pub fn can_transition_to(&self, next: Self) -> bool {
        if *self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Pending, Self::Running)
                | (Self::Pending, Self::AwaitingApproval)
                | (Self::Pending, Self::Failed)
                | (Self::Pending, Self::Cancelled)
                | (Self::Running, Self::AwaitingApproval)
                | (Self::Running, Self::Succeeded)
                | (Self::Running, Self::Failed)
                | (Self::Running, Self::Cancelled)
                | (Self::AwaitingApproval, Self::Running)
                | (Self::AwaitingApproval, Self::Succeeded)
                | (Self::AwaitingApproval, Self::Failed)
                | (Self::AwaitingApproval, Self::Cancelled)
        )
    }
}

/// A recorded agent task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentTask {
    /// Task identifier.
    pub id: String,
    /// Store the task operates on.
    pub store_id: StoreId,
    /// User that requested or owns the task.
    pub user_id: UserId,
    /// Current status.
    pub status: AgentTaskStatus,
    /// Provider identifier.
    pub provider_id: String,
    /// Model identifier.
    pub model: String,
    /// Serialized task input.
    pub input: serde_json::Value,
    /// Serialized task output, when available.
    pub output: Option<serde_json::Value>,
    /// Error message when failed.
    pub error: Option<String>,
    /// User who approved the task, when applicable.
    pub approved_by: Option<UserId>,
    /// Tools the task may invoke.
    pub tools: Vec<String>,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

/// An auditable task action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentAuditEntry {
    /// Entry identifier.
    pub id: String,
    /// Task identifier.
    pub task_id: String,
    /// Action name.
    pub action: String,
    /// User who performed the action.
    pub actor_id: Option<UserId>,
    /// JSON payload.
    pub data: serde_json::Value,
    /// Timestamp.
    pub created_at: i64,
}

/// Registry of providers, models and tools.
#[derive(Debug, Default)]
pub struct AiRegistry {
    providers: RwLock<HashMap<String, ProviderDefinition>>,
    models: RwLock<HashMap<String, ModelDefinition>>,
    tools: RwLock<HashMap<String, ToolDefinition>>,
}

impl AiRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a provider.
    pub fn register_provider(&self, provider: ProviderDefinition) -> Result<(), AiError> {
        if provider.id.trim().is_empty() {
            return Err(AiError::Invalid("provider id must not be empty".to_owned()));
        }
        let mut providers = self.providers.write().unwrap_or_else(|e| e.into_inner());
        if providers.contains_key(&provider.id) {
            return Err(AiError::Duplicate(provider.id));
        }
        providers.insert(provider.id.clone(), provider);
        Ok(())
    }

    /// Registers a model under an existing provider.
    pub fn register_model(&self, model: ModelDefinition) -> Result<(), AiError> {
        if model.id.trim().is_empty() {
            return Err(AiError::Invalid("model id must not be empty".to_owned()));
        }
        if !self
            .providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&model.provider_id)
        {
            return Err(AiError::UnknownProvider(model.provider_id));
        }
        let mut models = self.models.write().unwrap_or_else(|e| e.into_inner());
        if models.contains_key(&model.id) {
            return Err(AiError::Duplicate(model.id));
        }
        models.insert(model.id.clone(), model);
        Ok(())
    }

    /// Registers a tool definition.
    pub fn register_tool(&self, tool: ToolDefinition) -> Result<(), AiError> {
        if tool.name.trim().is_empty()
            || !tool
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(AiError::Invalid(format!(
                "invalid tool name: {}",
                tool.name
            )));
        }
        if !tool.input_schema.is_object() {
            return Err(AiError::Invalid(
                "tool input_schema must be a JSON object".to_owned(),
            ));
        }
        let mut tools = self.tools.write().unwrap_or_else(|e| e.into_inner());
        if tools.contains_key(&tool.name) {
            return Err(AiError::Duplicate(tool.name));
        }
        tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    /// Returns a provider.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<ProviderDefinition> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Returns a model.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<ModelDefinition> {
        self.models
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Returns a tool.
    #[must_use]
    pub fn tool(&self, name: &str) -> Option<ToolDefinition> {
        self.tools
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    /// Lists tool definitions sorted by name.
    #[must_use]
    pub fn tools(&self) -> Vec<ToolDefinition> {
        let mut tools: Vec<ToolDefinition> = self
            .tools
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        tools
    }

    /// Lists models sorted by id.
    #[must_use]
    pub fn models(&self) -> Vec<ModelDefinition> {
        let mut models: Vec<ModelDefinition> = self
            .models
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        models.sort_by(|left, right| left.id.cmp(&right.id));
        models
    }

    /// Ensures every tool exists.
    pub fn validate_tools(&self, names: &[String]) -> Result<(), AiError> {
        for name in names {
            if self.tool(name).is_none() {
                return Err(AiError::UnknownTool(name.clone()));
            }
        }
        Ok(())
    }
}

/// In-memory agent task store with an audit trail.
#[derive(Debug, Default)]
pub struct AgentTaskService {
    tasks: RwLock<HashMap<String, AgentTask>>,
    audit: Mutex<Vec<AgentAuditEntry>>,
    registry: AiRegistry,
}

impl AgentTaskService {
    /// Creates a service with its own registry.
    #[must_use]
    pub fn new(registry: AiRegistry) -> Self {
        Self {
            tasks: RwLock::new(HashMap::new()),
            audit: Mutex::new(Vec::new()),
            registry,
        }
    }

    /// Returns the AI registry.
    #[must_use]
    pub fn registry(&self) -> &AiRegistry {
        &self.registry
    }

    /// Creates a pending task.
    pub fn create_task(
        &self,
        store_id: StoreId,
        user_id: UserId,
        provider_id: &str,
        model: &str,
        input: serde_json::Value,
        tools: Vec<String>,
    ) -> Result<AgentTask, AiError> {
        let provider = self
            .registry
            .provider(provider_id)
            .ok_or_else(|| AiError::UnknownProvider(provider_id.to_owned()))?;
        if !provider.configured {
            return Err(AiError::Invalid(format!(
                "provider {provider_id} is not configured"
            )));
        }
        let definition = self
            .registry
            .model(model)
            .ok_or_else(|| AiError::NotFound(model.to_owned()))?;
        if definition.provider_id != provider_id {
            return Err(AiError::Invalid(
                "model does not belong to the provider".to_owned(),
            ));
        }
        if definition.supports_tools {
            self.registry.validate_tools(&tools)?;
        } else if !tools.is_empty() {
            return Err(AiError::Invalid("model does not support tools".to_owned()));
        }
        let now = now_timestamp();
        let task = AgentTask {
            id: format!("task_{}", uuid::Uuid::now_v7().simple()),
            store_id,
            user_id,
            status: AgentTaskStatus::Pending,
            provider_id: provider_id.to_owned(),
            model: model.to_owned(),
            input,
            output: None,
            error: None,
            approved_by: None,
            tools,
            created_at: now,
            updated_at: now,
        };
        self.tasks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.id.clone(), task.clone());
        self.record_audit(
            &task.id,
            "task.created",
            Some(&task.user_id),
            serde_json::json!({}),
        );
        Ok(task)
    }

    /// Returns a task.
    #[must_use]
    pub fn task(&self, id: &str) -> Option<AgentTask> {
        self.tasks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Lists tasks for a store, newest first.
    #[must_use]
    pub fn tasks_for_store(&self, store_id: &StoreId) -> Vec<AgentTask> {
        let mut tasks: Vec<AgentTask> = self
            .tasks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|task| &task.store_id == store_id)
            .cloned()
            .collect();
        tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at));
        tasks
    }

    /// Marks a task running.
    pub fn start(&self, id: &str, actor: Option<&UserId>) -> Result<AgentTask, AiError> {
        self.transition(id, AgentTaskStatus::Running, actor, "task.started")
    }

    /// Marks a task as awaiting human approval.
    pub fn require_approval(&self, id: &str) -> Result<AgentTask, AiError> {
        self.transition(
            id,
            AgentTaskStatus::AwaitingApproval,
            None,
            "task.approval_requested",
        )
    }

    /// Approves and resumes a task.
    pub fn approve(&self, id: &str, approver: &UserId) -> Result<AgentTask, AiError> {
        let mut task = self.transition(
            id,
            AgentTaskStatus::Running,
            Some(approver),
            "task.approved",
        )?;
        task.approved_by = Some(approver.clone());
        self.tasks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    /// Completes a task with output.
    pub fn succeed(&self, id: &str, output: serde_json::Value) -> Result<AgentTask, AiError> {
        let mut task = self
            .task(id)
            .ok_or_else(|| AiError::NotFound(id.to_owned()))?;
        if !task.status.can_transition_to(AgentTaskStatus::Succeeded) {
            return Err(AiError::InvalidTransition {
                from: task.status,
                to: AgentTaskStatus::Succeeded,
            });
        }
        task.status = AgentTaskStatus::Succeeded;
        task.output = Some(output);
        task.updated_at = now_timestamp();
        self.tasks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.id.clone(), task.clone());
        self.record_audit(id, "task.succeeded", None, serde_json::json!({}));
        Ok(task)
    }

    /// Fails a task with an error message.
    pub fn fail(&self, id: &str, error: &str) -> Result<AgentTask, AiError> {
        let mut task = self.transition(id, AgentTaskStatus::Failed, None, "task.failed")?;
        task.error = Some(error.to_owned());
        self.tasks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.id.clone(), task.clone());
        Ok(task)
    }

    /// Cancels a task.
    pub fn cancel(&self, id: &str, actor: Option<&UserId>) -> Result<AgentTask, AiError> {
        self.transition(id, AgentTaskStatus::Cancelled, actor, "task.cancelled")
    }

    /// Returns the audit trail for a task.
    #[must_use]
    pub fn audit_for(&self, task_id: &str) -> Vec<AgentAuditEntry> {
        self.audit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|entry| entry.task_id == task_id)
            .cloned()
            .collect()
    }

    fn transition(
        &self,
        id: &str,
        next: AgentTaskStatus,
        actor: Option<&UserId>,
        action: &str,
    ) -> Result<AgentTask, AiError> {
        let mut task = self
            .task(id)
            .ok_or_else(|| AiError::NotFound(id.to_owned()))?;
        if !task.status.can_transition_to(next) {
            return Err(AiError::InvalidTransition {
                from: task.status,
                to: next,
            });
        }
        task.status = next;
        task.updated_at = now_timestamp();
        self.tasks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.id.clone(), task.clone());
        self.record_audit(id, action, actor, serde_json::json!({}));
        Ok(task)
    }

    fn record_audit(
        &self,
        task_id: &str,
        action: &str,
        actor_id: Option<&UserId>,
        data: serde_json::Value,
    ) {
        let entry = AgentAuditEntry {
            id: format!("aiaudit_{}", uuid::Uuid::now_v7().simple()),
            task_id: task_id.to_owned(),
            action: action.to_owned(),
            actor_id: actor_id.cloned(),
            data,
            created_at: now_timestamp(),
        };
        self.audit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry);
    }
}

#[cfg(test)]
mod tests {
    use fastcart_kernel::{StoreId, UserId};

    use super::{
        AgentTaskService, AgentTaskStatus, AiError, AiRegistry, ModelDefinition,
        ProviderDefinition, ToolDefinition,
    };

    fn registry() -> AiRegistry {
        let registry = AiRegistry::new();
        registry
            .register_provider(ProviderDefinition {
                id: "anthropic".to_owned(),
                name: "Anthropic".to_owned(),
                configured: true,
            })
            .unwrap();
        registry
            .register_model(ModelDefinition {
                id: "claude-sonnet-5-5".to_owned(),
                provider_id: "anthropic".to_owned(),
                name: "Claude Sonnet 5.5".to_owned(),
                context_window: 200_000,
                supports_tools: true,
            })
            .unwrap();
        registry
            .register_tool(ToolDefinition {
                name: "list_products".to_owned(),
                description: "List products".to_owned(),
                input_schema: serde_json::json!({"type": "object"}),
                mutates: false,
                requires_approval: false,
            })
            .unwrap();
        registry
    }

    #[test]
    fn registries_validate_definitions() {
        let registry = registry();
        assert!(registry.provider("anthropic").is_some());
        assert!(registry.model("claude-sonnet-5-5").is_some());
        assert_eq!(registry.tools().len(), 1);

        assert_eq!(
            registry.register_model(ModelDefinition {
                id: "orphan".to_owned(),
                provider_id: "missing".to_owned(),
                name: "Orphan".to_owned(),
                context_window: 1,
                supports_tools: false,
            }),
            Err(AiError::UnknownProvider("missing".to_owned()))
        );
    }

    #[test]
    fn tasks_follow_the_state_machine() {
        let service = AgentTaskService::new(registry());
        let store = StoreId::new();
        let user = UserId::new();
        let task = service
            .create_task(
                store.clone(),
                user.clone(),
                "anthropic",
                "claude-sonnet-5-5",
                serde_json::json!({"prompt": "hi"}),
                vec!["list_products".to_owned()],
            )
            .unwrap();
        assert_eq!(task.status, AgentTaskStatus::Pending);

        let running = service.start(&task.id, Some(&user)).unwrap();
        assert_eq!(running.status, AgentTaskStatus::Running);

        let done = service
            .succeed(&task.id, serde_json::json!({"ok": true}))
            .unwrap();
        assert_eq!(done.status, AgentTaskStatus::Succeeded);
        assert!(service.fail(&task.id, "late").is_err());
        assert!(service.audit_for(&task.id).len() >= 3);
        assert_eq!(service.tasks_for_store(&store).len(), 1);
    }

    #[test]
    fn approval_resumes_a_waiting_task() {
        let service = AgentTaskService::new(registry());
        let task = service
            .create_task(
                StoreId::new(),
                UserId::new(),
                "anthropic",
                "claude-sonnet-5-5",
                serde_json::json!({}),
                vec![],
            )
            .unwrap();
        service.require_approval(&task.id).unwrap();
        assert_eq!(
            service.task(&task.id).unwrap().status,
            AgentTaskStatus::AwaitingApproval
        );
        let approver = UserId::new();
        let resumed = service.approve(&task.id, &approver).unwrap();
        assert_eq!(resumed.status, AgentTaskStatus::Running);
        assert_eq!(resumed.approved_by, Some(approver));
    }

    #[test]
    fn unknown_tools_and_unconfigured_providers_are_rejected() {
        let service = AgentTaskService::new(registry());
        let error = service
            .create_task(
                StoreId::new(),
                UserId::new(),
                "anthropic",
                "claude-sonnet-5-5",
                serde_json::json!({}),
                vec!["delete_everything".to_owned()],
            )
            .unwrap_err();
        assert_eq!(error, AiError::UnknownTool("delete_everything".to_owned()));

        let registry = AiRegistry::new();
        registry
            .register_provider(ProviderDefinition {
                id: "p".to_owned(),
                name: "P".to_owned(),
                configured: false,
            })
            .unwrap();
        registry
            .register_model(ModelDefinition {
                id: "m".to_owned(),
                provider_id: "p".to_owned(),
                name: "M".to_owned(),
                context_window: 1,
                supports_tools: false,
            })
            .unwrap();
        let service = AgentTaskService::new(registry);
        assert!(
            service
                .create_task(
                    StoreId::new(),
                    UserId::new(),
                    "p",
                    "m",
                    serde_json::json!({}),
                    vec![],
                )
                .is_err()
        );
    }
}
