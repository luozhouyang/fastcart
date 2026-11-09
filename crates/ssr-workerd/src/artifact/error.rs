//! Artifact validation and storage errors.

use std::path::PathBuf;

/// Errors raised while validating or storing an SSR artifact.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// The manifest is missing or unreadable.
    #[error("artifact manifest not found or unreadable at {0}")]
    MissingManifest(PathBuf),
    /// The manifest is not valid JSON.
    #[error("invalid artifact manifest JSON: {0}")]
    InvalidJson(String),
    /// A manifest field is invalid.
    #[error("invalid artifact manifest: {0}")]
    InvalidManifest(String),
    /// A path escapes the artifact root.
    #[error("unsafe artifact path: {0}")]
    UnsafePath(String),
    /// The worker entry file is missing.
    #[error("artifact entry not found: {0}")]
    MissingEntry(String),
    /// The client asset directory is missing.
    #[error("artifact asset directory not found: {0}")]
    MissingAssets(String),
    /// The artifact exceeds a configured limit.
    #[error("artifact exceeds {kind} limit: {actual} > {limit}")]
    ExceedsLimit {
        /// The limit name, such as `bytes` or `files`.
        kind: &'static str,
        /// Observed value.
        actual: u64,
        /// Configured maximum.
        limit: u64,
    },
    /// The tree contains a symlink, which is not allowed.
    #[error("artifact contains a symlink: {0}")]
    Symlink(String),
    /// A digest string is malformed.
    #[error("invalid artifact digest: {0}")]
    InvalidDigest(String),
    /// Stored content does not match its digest.
    #[error("artifact digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Expected digest.
        expected: String,
        /// Recomputed digest.
        actual: String,
    },
    /// A staging or destination path already exists with different content.
    #[error("artifact already exists at {0}")]
    Conflict(PathBuf),
    /// Filesystem failure with context.
    #[error("artifact filesystem error at {path}: {message}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying error message.
        message: String,
    },
}
