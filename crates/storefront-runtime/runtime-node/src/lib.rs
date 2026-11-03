//! Node.js sidecar storefront runtime.
//!
//! This adapter is the documented fallback when embedded V8 cannot support a
//! required TanStack Start capability. Phase 0 defines the adapter boundary only.

use fastcart_runtime_api::{RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime};

/// Node.js sidecar runtime adapter.
#[derive(Clone, Debug)]
pub struct NodeRuntime {
    node_path: String,
}

impl NodeRuntime {
    /// Creates a sidecar adapter using the given Node.js executable.
    #[must_use]
    pub fn new(node_path: impl Into<String>) -> Self {
        Self {
            node_path: node_path.into(),
        }
    }

    /// Returns the configured Node.js executable path.
    #[must_use]
    pub fn node_path(&self) -> &str {
        &self.node_path
    }
}

#[async_trait::async_trait]
impl StorefrontRuntime for NodeRuntime {
    async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        Err(RuntimeError::NotInitialized(
            "node sidecar adapter is not implemented yet".to_owned(),
        ))
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::NotInitialized(
            "node sidecar adapter is not implemented yet".to_owned(),
        ))
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
