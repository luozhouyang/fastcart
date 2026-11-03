//! Embedded V8 storefront runtime.
//!
//! Phase 0 provides the adapter type and the stable runtime boundary. Phase 1
//! wires deno_core, V8 platform initialization, module loading and the web API
//! extensions behind this same interface.

use fastcart_runtime_api::{RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime};

/// Embedded V8 runtime adapter.
#[derive(Debug, Default)]
pub struct V8Runtime;

impl V8Runtime {
    /// Creates an uninitialized embedded runtime.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl StorefrontRuntime for V8Runtime {
    async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        Err(RuntimeError::NotInitialized(
            "deno_core integration starts in Phase 1".to_owned(),
        ))
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        Err(RuntimeError::NotInitialized(
            "deno_core integration starts in Phase 1".to_owned(),
        ))
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
