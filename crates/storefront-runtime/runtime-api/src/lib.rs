//! Storefront runtime contracts shared by every runtime adapter.
//!
//! The runtime API is deliberately independent of deno_core, V8, Node.js and
//! TanStack Start. Axum and the commerce core depend only on these types.

pub mod error;
pub mod types;

pub use error::RuntimeError;
pub use types::{
    CapabilityGrant, PageContext, RuntimeBody, RuntimeMode, RuntimeRequest, RuntimeResponse,
    RuntimeTheme,
};

/// Executes storefront server bundles.
#[async_trait::async_trait]
pub trait StorefrontRuntime: Send + Sync {
    /// Renders one storefront request.
    async fn render(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError>;

    /// Reports whether the runtime is ready to serve traffic.
    async fn health_check(&self) -> Result<(), RuntimeError>;

    /// Stops the runtime and releases its resources.
    async fn shutdown(&self) -> Result<(), RuntimeError>;
}
