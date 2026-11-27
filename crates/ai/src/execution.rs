//! Deterministic tool execution.
//!
//! A tool invocation is validated against the registry's JSON Schema, parsed
//! into a typed input and dispatched to a [`ToolRuntime`] that binds the
//! operation to application services. This crate stays independent of the
//! commerce modules: the runtime implementation lives in the API layer and
//! calls existing services, never repositories or SQLite directly.
//!
//! No model is called here. A real provider client is a documented follow-up;
//! [`crate::provider::ModelProvider`] fixes that boundary.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AiRegistry, ToolDefinition, tool_names};

/// Structured failure of one tool invocation.
///
/// The variant is the machine-readable part of the error that the task store
/// persists; the `Display` text is the human-readable part.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ToolExecutionError {
    /// The tool name is not registered.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    /// The input does not match the tool's JSON Schema or typed shape.
    #[error("{0}")]
    InvalidInput(String),
    /// The registered tool schema itself cannot be compiled.
    #[error("invalid tool schema: {0}")]
    InvalidSchema(String),
    /// The addressed resource does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The tool ran but failed, for example on a state conflict or a database
    /// error.
    #[error("{0}")]
    Failed(String),
}

impl ToolExecutionError {
    /// Stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownTool(_) => "UNKNOWN_TOOL",
            Self::InvalidInput(_) => "VALIDATION_ERROR",
            Self::InvalidSchema(_) => "INVALID_TOOL_SCHEMA",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Failed(_) => "TOOL_ERROR",
        }
    }

    /// Renders the error as the structured JSON document persisted in
    /// `ai_tasks.error`.
    #[must_use]
    pub fn to_stored_json(&self) -> String {
        serde_json::json!({
            "code": self.code(),
            "message": self.to_string(),
        })
        .to_string()
    }
}

/// Input of `page.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageGetInput {
    /// Page identifier.
    pub id: String,
}

/// Input of `page.update`. Omitted fields keep their current value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageUpdateInput {
    /// Page identifier.
    pub id: String,
    /// Replacement title.
    #[serde(default)]
    pub title: Option<String>,
    /// Replacement handle.
    #[serde(default)]
    pub handle: Option<String>,
    /// Replacement page classifier.
    #[serde(default)]
    pub page_type: Option<String>,
    /// Replacement structured body.
    #[serde(default)]
    pub body: Option<Value>,
    /// Replacement structured SEO metadata.
    #[serde(default)]
    pub seo: Option<Value>,
    /// Revision the caller last observed; stale values are rejected.
    #[serde(default)]
    pub base_revision: Option<i64>,
}

/// Input of `page.publish`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PagePublishInput {
    /// Page identifier.
    pub id: String,
}

/// Input of `block.list_types`; the tool takes no arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockListTypesInput {}

/// Input of `order.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderGetInput {
    /// Order identifier.
    pub id: String,
}

/// A validated invocation of one built-in tool.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolInvocation {
    /// Read one content page.
    PageGet(PageGetInput),
    /// Update one content page.
    PageUpdate(PageUpdateInput),
    /// Publish one content page.
    PagePublish(PagePublishInput),
    /// List the block types available to page bodies.
    BlockListTypes(BlockListTypesInput),
    /// Read one order aggregate.
    OrderGet(OrderGetInput),
}

impl ToolInvocation {
    /// Stable tool name of this invocation.
    #[must_use]
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::PageGet(_) => tool_names::PAGE_GET,
            Self::PageUpdate(_) => tool_names::PAGE_UPDATE,
            Self::PagePublish(_) => tool_names::PAGE_PUBLISH,
            Self::BlockListTypes(_) => tool_names::BLOCK_LIST_TYPES,
            Self::OrderGet(_) => tool_names::ORDER_GET,
        }
    }

    /// Parses a typed invocation. The input must already have passed JSON
    /// Schema validation; this second pass produces the typed shape the
    /// runtime consumes.
    pub fn parse(tool: &str, input: &Value) -> Result<Self, ToolExecutionError> {
        match tool {
            tool_names::PAGE_GET => Ok(Self::PageGet(parse_input(tool, input)?)),
            tool_names::PAGE_UPDATE => Ok(Self::PageUpdate(parse_input(tool, input)?)),
            tool_names::PAGE_PUBLISH => Ok(Self::PagePublish(parse_input(tool, input)?)),
            tool_names::BLOCK_LIST_TYPES => Ok(Self::BlockListTypes(parse_input(tool, input)?)),
            tool_names::ORDER_GET => Ok(Self::OrderGet(parse_input(tool, input)?)),
            other => Err(ToolExecutionError::UnknownTool(other.to_owned())),
        }
    }
}

fn parse_input<T: DeserializeOwned>(tool: &str, input: &Value) -> Result<T, ToolExecutionError> {
    serde_json::from_value(input.clone()).map_err(|error| {
        ToolExecutionError::InvalidInput(format!("tool {tool} input is invalid: {error}"))
    })
}

/// Validates `input` against the registered JSON Schema for `tool`.
///
/// Returns the definition on success so callers can inspect
/// [`ToolDefinition::requires_approval`] without a second lookup.
pub fn validate_tool_input(
    registry: &AiRegistry,
    tool: &str,
    input: &Value,
) -> Result<ToolDefinition, ToolExecutionError> {
    let definition = registry
        .tool(tool)
        .ok_or_else(|| ToolExecutionError::UnknownTool(tool.to_owned()))?;
    let validator = jsonschema::validator_for(&definition.input_schema).map_err(|error| {
        ToolExecutionError::InvalidSchema(format!("tool {tool} schema: {error}"))
    })?;
    validator.validate(input).map_err(|error| {
        ToolExecutionError::InvalidInput(format!("tool {tool} input is invalid: {error}"))
    })?;
    Ok(definition)
}

/// Application services backing the built-in tools.
///
/// Implementations translate one typed invocation into an existing
/// application-service call and return the tool output. Errors are already
/// structured, so a runtime should map `AppError::NotFound` to
/// [`ToolExecutionError::NotFound`] and keep the rest as
/// [`ToolExecutionError::Failed`].
#[async_trait]
pub trait ToolRuntime: Send + Sync {
    /// Implements `page.get`.
    async fn get_page(&self, input: PageGetInput) -> Result<Value, ToolExecutionError>;
    /// Implements `page.update`.
    async fn update_page(&self, input: PageUpdateInput) -> Result<Value, ToolExecutionError>;
    /// Implements `page.publish`.
    async fn publish_page(&self, input: PagePublishInput) -> Result<Value, ToolExecutionError>;
    /// Implements `block.list_types`.
    async fn list_block_types(
        &self,
        input: BlockListTypesInput,
    ) -> Result<Value, ToolExecutionError>;
    /// Implements `order.get`.
    async fn get_order(&self, input: OrderGetInput) -> Result<Value, ToolExecutionError>;
}

/// Validates, parses and dispatches one tool invocation.
pub struct ToolExecutor<'a, R: ToolRuntime + ?Sized> {
    registry: &'a AiRegistry,
    runtime: &'a R,
}

impl<'a, R: ToolRuntime + ?Sized> ToolExecutor<'a, R> {
    /// Creates an executor over a registry and a runtime.
    #[must_use]
    pub fn new(registry: &'a AiRegistry, runtime: &'a R) -> Self {
        Self { registry, runtime }
    }

    /// Validates the input against the tool's JSON Schema and parses it.
    pub fn prepare(&self, tool: &str, input: &Value) -> Result<ToolInvocation, ToolExecutionError> {
        validate_tool_input(self.registry, tool, input)?;
        ToolInvocation::parse(tool, input)
    }

    /// Runs one tool end to end.
    pub async fn execute(&self, tool: &str, input: &Value) -> Result<Value, ToolExecutionError> {
        let invocation = self.prepare(tool, input)?;
        self.run(invocation).await
    }

    /// Runs an already prepared invocation.
    pub async fn run(&self, invocation: ToolInvocation) -> Result<Value, ToolExecutionError> {
        match invocation {
            ToolInvocation::PageGet(input) => self.runtime.get_page(input).await,
            ToolInvocation::PageUpdate(input) => self.runtime.update_page(input).await,
            ToolInvocation::PagePublish(input) => self.runtime.publish_page(input).await,
            ToolInvocation::BlockListTypes(input) => self.runtime.list_block_types(input).await,
            ToolInvocation::OrderGet(input) => self.runtime.get_order(input).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        BlockListTypesInput, OrderGetInput, PageGetInput, PagePublishInput, PageUpdateInput,
        ToolExecutionError, ToolExecutor, ToolInvocation, ToolRuntime, validate_tool_input,
    };
    use crate::{AiRegistry, builtin_registry, tool_names};

    /// Runtime double that echoes the parsed input and returns a fixed value.
    #[derive(Default)]
    struct MockRuntime {
        calls: std::sync::Mutex<Vec<&'static str>>,
        failure: Option<ToolExecutionError>,
    }

    impl MockRuntime {
        fn failing(error: ToolExecutionError) -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
                failure: Some(error),
            }
        }

        fn record(&self, name: &'static str) {
            self.calls.lock().unwrap().push(name);
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }

        fn checked(&self) -> Result<(), ToolExecutionError> {
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ToolRuntime for MockRuntime {
        async fn get_page(
            &self,
            input: PageGetInput,
        ) -> Result<serde_json::Value, ToolExecutionError> {
            self.record(tool_names::PAGE_GET);
            self.checked()?;
            Ok(json!({"id": input.id}))
        }

        async fn update_page(
            &self,
            input: PageUpdateInput,
        ) -> Result<serde_json::Value, ToolExecutionError> {
            self.record(tool_names::PAGE_UPDATE);
            self.checked()?;
            Ok(json!({"id": input.id, "title": input.title}))
        }

        async fn publish_page(
            &self,
            input: PagePublishInput,
        ) -> Result<serde_json::Value, ToolExecutionError> {
            self.record(tool_names::PAGE_PUBLISH);
            self.checked()?;
            Ok(json!({"id": input.id}))
        }

        async fn list_block_types(
            &self,
            _input: BlockListTypesInput,
        ) -> Result<serde_json::Value, ToolExecutionError> {
            self.record(tool_names::BLOCK_LIST_TYPES);
            self.checked()?;
            Ok(json!(["Hero"]))
        }

        async fn get_order(
            &self,
            input: OrderGetInput,
        ) -> Result<serde_json::Value, ToolExecutionError> {
            self.record(tool_names::ORDER_GET);
            self.checked()?;
            Ok(json!({"id": input.id}))
        }
    }

    fn registry() -> AiRegistry {
        builtin_registry()
    }

    #[test]
    fn schema_validation_accepts_and_rejects() {
        validate_tool_input(&registry(), tool_names::PAGE_GET, &json!({"id": "page_1"}))
            .expect("valid input");
        let error = validate_tool_input(&registry(), tool_names::PAGE_GET, &json!({}))
            .expect_err("missing id");
        assert_eq!(error.code(), "VALIDATION_ERROR");
        let error = validate_tool_input(
            &registry(),
            tool_names::PAGE_GET,
            &json!({"id": "page_1", "extra": true}),
        )
        .expect_err("additional properties are rejected");
        assert_eq!(error.code(), "VALIDATION_ERROR");

        validate_tool_input(&registry(), tool_names::BLOCK_LIST_TYPES, &json!({}))
            .expect("empty object is valid");
        let error = validate_tool_input(
            &registry(),
            tool_names::PAGE_UPDATE,
            &json!({"id": "page_1", "base_revision": "one"}),
        )
        .expect_err("wrong type");
        assert_eq!(error.code(), "VALIDATION_ERROR");
    }

    #[test]
    fn unknown_tools_are_rejected_by_validation_and_parsing() {
        let error = validate_tool_input(&registry(), "page.delete", &json!({"id": "page_1"}))
            .expect_err("unknown tool");
        assert_eq!(
            error,
            ToolExecutionError::UnknownTool("page.delete".to_owned())
        );
        assert_eq!(error.code(), "UNKNOWN_TOOL");
        assert_eq!(
            ToolInvocation::parse("page.delete", &json!({})).unwrap_err(),
            ToolExecutionError::UnknownTool("page.delete".to_owned())
        );
    }

    #[test]
    fn typed_parsing_keeps_omitted_update_fields_unset() {
        let invocation = ToolInvocation::parse(
            tool_names::PAGE_UPDATE,
            &json!({"id": "page_1", "title": "New"}),
        )
        .expect("valid input");
        assert_eq!(
            invocation,
            ToolInvocation::PageUpdate(PageUpdateInput {
                id: "page_1".to_owned(),
                title: Some("New".to_owned()),
                ..PageUpdateInput::default()
            })
        );
        assert_eq!(invocation.tool_name(), tool_names::PAGE_UPDATE);

        assert_eq!(
            ToolInvocation::parse(tool_names::BLOCK_LIST_TYPES, &json!({})).unwrap(),
            ToolInvocation::BlockListTypes(BlockListTypesInput {})
        );
        assert_eq!(
            ToolInvocation::parse(tool_names::ORDER_GET, &json!({"id": "order_1"})).unwrap(),
            ToolInvocation::OrderGet(OrderGetInput {
                id: "order_1".to_owned()
            })
        );
    }

    #[test]
    fn typed_parsing_rejects_shapes_the_schema_would_reject() {
        let error =
            ToolInvocation::parse(tool_names::PAGE_PUBLISH, &json!({})).expect_err("missing id");
        assert_eq!(error.code(), "VALIDATION_ERROR");
        let error = ToolInvocation::parse(
            tool_names::PAGE_GET,
            &json!({"id": "page_1", "extra": true}),
        )
        .expect_err("unknown field");
        assert_eq!(error.code(), "VALIDATION_ERROR");
    }

    #[tokio::test]
    async fn executor_dispatches_each_builtin_tool() {
        let registry = registry();
        let runtime = MockRuntime::default();
        let executor = ToolExecutor::new(&registry, &runtime);

        assert_eq!(
            executor
                .execute(tool_names::PAGE_GET, &json!({"id": "page_1"}))
                .await
                .unwrap(),
            json!({"id": "page_1"})
        );
        assert_eq!(
            executor
                .execute(
                    tool_names::PAGE_UPDATE,
                    &json!({"id": "page_1", "title": "T"})
                )
                .await
                .unwrap(),
            json!({"id": "page_1", "title": "T"})
        );
        assert_eq!(
            executor
                .execute(tool_names::PAGE_PUBLISH, &json!({"id": "page_1"}))
                .await
                .unwrap(),
            json!({"id": "page_1"})
        );
        assert_eq!(
            executor
                .execute(tool_names::BLOCK_LIST_TYPES, &json!({}))
                .await
                .unwrap(),
            json!(["Hero"])
        );
        assert_eq!(
            executor
                .execute(tool_names::ORDER_GET, &json!({"id": "order_1"}))
                .await
                .unwrap(),
            json!({"id": "order_1"})
        );
        assert_eq!(runtime.calls().len(), 5);
    }

    #[tokio::test]
    async fn executor_does_not_run_a_tool_with_invalid_input() {
        let registry = registry();
        let runtime = MockRuntime::default();
        let executor = ToolExecutor::new(&registry, &runtime);
        let error = executor
            .execute(tool_names::PAGE_GET, &json!({}))
            .await
            .expect_err("invalid input");
        assert_eq!(error.code(), "VALIDATION_ERROR");
        assert!(runtime.calls().is_empty(), "the runtime must not be called");
    }

    #[tokio::test]
    async fn executor_propagates_structured_runtime_errors() {
        let registry = registry();
        let runtime = MockRuntime::failing(ToolExecutionError::NotFound(
            "page not found: page_9".to_owned(),
        ));
        let executor = ToolExecutor::new(&registry, &runtime);
        let error = executor
            .execute(tool_names::PAGE_GET, &json!({"id": "page_9"}))
            .await
            .expect_err("runtime failure");
        assert_eq!(error.code(), "NOT_FOUND");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&error.to_stored_json()).unwrap(),
            json!({"code": "NOT_FOUND", "message": "page not found: page_9"})
        );
    }
}
