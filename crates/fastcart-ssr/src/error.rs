//! Errors raised by the SSR runtime manager and gateway.

use std::path::PathBuf;
use std::time::Duration;

/// Errors raised while starting, probing or proxying to workerd.
#[derive(Debug, thiserror::Error)]
pub enum SsrError {
    /// The configured workerd binary does not exist.
    #[error("workerd binary not found: {0}")]
    BinaryNotFound(PathBuf),
    /// The configured Cap'n Proto file does not exist.
    #[error("workerd config not found: {0}")]
    ConfigNotFound(PathBuf),
    /// A configuration value is invalid.
    #[error("invalid workerd configuration: {0}")]
    InvalidConfig(String),
    /// The child process could not be spawned.
    #[error("failed to start workerd: {0}")]
    Start(String),
    /// The process exited before it became ready.
    #[error("workerd exited before becoming ready: {0}")]
    EarlyExit(String),
    /// The process did not answer its health check in time.
    #[error("workerd did not become ready within {0:?}")]
    StartupTimeout(Duration),
    /// The process is not running.
    #[error("workerd is not running")]
    NotRunning,
    /// The health probe failed.
    #[error("workerd health check failed: {0}")]
    Health(String),
    /// The gateway request is malformed.
    #[error("invalid SSR request: {0}")]
    InvalidRequest(String),
    /// The gateway request failed before a response was received.
    #[error("SSR request failed: {0}")]
    Request(String),
    /// A response stream failed after headers were received.
    #[error("SSR response failed: {0}")]
    Response(String),
    /// The process did not stop within the configured deadline.
    #[error("workerd did not stop within {0:?}")]
    ShutdownTimeout(Duration),
    /// The requested runtime group is unknown.
    #[error("unknown runtime group: {0}")]
    UnknownGroup(String),
    /// The runtime group already has a running process.
    #[error("runtime group is already running: {0}")]
    AlreadyRunning(String),
    /// The group's restart limit was reached.
    #[error("runtime group restart limit reached: {0}")]
    RestartLimit(String),
    /// An artifact cannot be represented as a workerd configuration.
    #[error("invalid runtime configuration: {0}")]
    InvalidRuntimeConfig(String),
}
