//! Model provider boundary.
//!
//! Tool execution is deterministic and provider-independent: a task records a
//! tool and its input, and the executor runs it without calling a model. This
//! module fixes the boundary for the next step — an agent loop that asks a
//! model which tool to call — and ships a deterministic test double.
//!
//! A production Anthropic or OpenAI client is a documented follow-up. It must
//! implement [`ModelProvider`], keep credentials in the host configuration and
//! never bypass the registry schema or the approval gate.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Role of one message in a model conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    /// Host instructions.
    System,
    /// Human turn.
    User,
    /// Model turn.
    Assistant,
}

/// One message in a model request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelMessage {
    /// Message author.
    pub role: ModelRole,
    /// Plain-text content.
    pub content: String,
}

impl ModelMessage {
    /// Creates a message.
    #[must_use]
    pub fn new(role: ModelRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// One tool call requested by a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// Stable registered tool name.
    pub tool: String,
    /// Unvalidated JSON input; the executor validates it before running.
    pub input: Value,
}

/// A request sent to a model provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// Registered model identifier.
    pub model: String,
    /// Conversation so far.
    pub messages: Vec<ModelMessage>,
    /// Tool names exposed to the model, in registry order.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// One model turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    /// Assistant text; empty when the model only calls tools.
    #[serde(default)]
    pub content: String,
    /// Tool calls requested in this turn.
    #[serde(default)]
    pub tool_calls: Vec<ToolCallRequest>,
}

/// Provider failures.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelProviderError {
    /// Credentials are missing or the provider is disabled.
    #[error("provider {0} is not configured")]
    NotConfigured(String),
    /// The provider rejected the request or was unreachable.
    #[error("model request failed: {0}")]
    Failed(String),
}

/// A source of model turns.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Provider identifier matching a registered provider definition.
    fn provider_id(&self) -> &str;

    /// Produces the next assistant turn.
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ModelProviderError>;
}

/// Deterministic provider used by tests.
///
/// Responses are returned in order and removed as they are consumed, so a test
/// can script a tool call followed by a final answer and assert the loop's
/// behaviour without a network call.
#[derive(Debug)]
pub struct MockModelProvider {
    provider_id: String,
    responses: Mutex<VecDeque<ModelResponse>>,
}

impl MockModelProvider {
    /// Creates a provider that serves `responses` in order.
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        responses: impl IntoIterator<Item = ModelResponse>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    /// Creates a provider that serves one response.
    #[must_use]
    pub fn with_response(provider_id: impl Into<String>, response: ModelResponse) -> Self {
        Self::new(provider_id, [response])
    }

    /// Creates a provider that requests one tool call on its first turn.
    #[must_use]
    pub fn scripted_tool_call(
        provider_id: impl Into<String>,
        tool: impl Into<String>,
        input: Value,
    ) -> Self {
        Self::with_response(
            provider_id,
            ModelResponse {
                content: String::new(),
                tool_calls: vec![ToolCallRequest {
                    tool: tool.into(),
                    input,
                }],
            },
        )
    }

    /// Number of scripted responses that have not been served yet.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.responses
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }
}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    async fn complete(&self, _request: ModelRequest) -> Result<ModelResponse, ModelProviderError> {
        self.responses
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop_front()
            .ok_or_else(|| {
                ModelProviderError::Failed("mock provider has no scripted response".to_owned())
            })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        MockModelProvider, ModelMessage, ModelProvider, ModelProviderError, ModelResponse,
        ModelRole, ToolCallRequest,
    };

    fn request() -> super::ModelRequest {
        super::ModelRequest {
            model: "claude-sonnet-5-5".to_owned(),
            messages: vec![ModelMessage::new(ModelRole::User, "count the pages")],
            tools: vec!["page.get".to_owned()],
        }
    }

    #[tokio::test]
    async fn mock_provider_serves_scripted_responses_in_order() {
        let provider = MockModelProvider::new(
            "anthropic",
            [
                ModelResponse {
                    content: String::new(),
                    tool_calls: vec![ToolCallRequest {
                        tool: "page.get".to_owned(),
                        input: json!({"id": "page_1"}),
                    }],
                },
                ModelResponse {
                    content: "There is one page.".to_owned(),
                    tool_calls: Vec::new(),
                },
            ],
        );
        assert_eq!(provider.provider_id(), "anthropic");

        let first = provider.complete(request()).await.unwrap();
        assert_eq!(first.tool_calls.len(), 1);
        assert_eq!(first.tool_calls[0].tool, "page.get");

        let second = provider.complete(request()).await.unwrap();
        assert_eq!(second.content, "There is one page.");
        assert_eq!(provider.remaining(), 0);

        assert_eq!(
            provider.complete(request()).await.unwrap_err(),
            ModelProviderError::Failed("mock provider has no scripted response".to_owned())
        );
    }

    #[tokio::test]
    async fn scripted_tool_call_builds_one_response() {
        let provider =
            MockModelProvider::scripted_tool_call("anthropic", "page.publish", json!({"id": "p"}));
        let response = provider.complete(request()).await.unwrap();
        assert_eq!(
            response.tool_calls,
            vec![ToolCallRequest {
                tool: "page.publish".to_owned(),
                input: json!({"id": "p"}),
            }]
        );
        assert_eq!(provider.remaining(), 0);
    }
}
