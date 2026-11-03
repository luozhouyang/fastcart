//! AI extension foundation.
//!
//! AI tools invoke commerce application services only. They never access
//! repositories or SQLite directly, and mutations remain auditable.

use fastcart_kernel::{StoreId, UserId};

/// A controlled operation exposed to agents.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    /// Stable tool name.
    pub name: String,
    /// Description presented to the model.
    pub description: String,
    /// JSON Schema for tool input.
    pub input_schema: serde_json::Value,
}

/// Lifecycle state of an agent task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// A recorded agent task.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AgentTask {
    /// Task identifier.
    pub id: String,
    /// Store the task operates on.
    pub store_id: StoreId,
    /// User that requested or owns the task.
    pub user_id: UserId,
    /// Current status.
    pub status: AgentTaskStatus,
    /// Model identifier used for the task.
    pub model: String,
    /// Serialized task input.
    pub input: serde_json::Value,
    /// Serialized task output, when available.
    pub output: Option<serde_json::Value>,
}
