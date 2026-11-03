//! Runtime error categories.

/// Errors raised by storefront runtime adapters.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The runtime has not been initialized yet.
    #[error("runtime not initialized: {0}")]
    NotInitialized(String),
    /// A module in the server bundle could not be loaded.
    #[error("module load failed: {0}")]
    ModuleLoad(String),
    /// JavaScript execution failed.
    #[error("javascript execution failed: {0}")]
    JsExecution(String),
    /// The internal fetch bridge failed.
    #[error("fetch bridge failed: {0}")]
    FetchBridge(String),
    /// A response stream failed.
    #[error("stream failed: {0}")]
    Stream(String),
    /// The request exceeded its timeout.
    #[error("request timed out after {0} ms")]
    Timeout(u64),
    /// The request was cancelled.
    #[error("request cancelled")]
    Cancelled,
    /// The runtime exceeded its memory limit.
    #[error("runtime out of memory")]
    OutOfMemory,
    /// No worker could accept the request.
    #[error("worker unavailable: {0}")]
    WorkerUnavailable(String),
    /// A pool was created without workers.
    #[error("runtime pool is empty")]
    EmptyPool,
}
