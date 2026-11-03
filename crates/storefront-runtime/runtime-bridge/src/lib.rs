//! Internal fetch bridge between the storefront runtime and commerce services.

use bytes::Bytes;
use fastcart_kernel::StoreContext;

/// Logical prefix for internal storefront routes.
pub const INTERNAL_STOREFRONT_PREFIX: &str = "/internal/storefront";

/// A request dispatched from the JavaScript runtime to Rust.
#[derive(Debug)]
pub struct BridgeRequest {
    /// Trusted store context resolved by the host.
    pub store: StoreContext,
    /// HTTP method.
    pub method: String,
    /// Logical path including the internal prefix.
    pub path: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// Request body.
    pub body: Bytes,
}

/// A response returned to the JavaScript runtime.
#[derive(Debug)]
pub struct BridgeResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Bytes,
}

/// Errors raised by the internal fetch bridge.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// No internal route matched the request.
    #[error("internal route not found: {0}")]
    NotFound(String),
    /// The bridge failed while dispatching the request.
    #[error("bridge failure: {0}")]
    Internal(String),
}

/// Dispatches internal storefront requests without TCP or public HTTP.
#[async_trait::async_trait]
pub trait InternalFetch: Send + Sync {
    /// Dispatches one internal request.
    async fn fetch(&self, request: BridgeRequest) -> Result<BridgeResponse, BridgeError>;
}
