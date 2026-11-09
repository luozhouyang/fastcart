//! Backend contract for credential persistence.

use async_trait::async_trait;

use crate::model::CredentialSnapshot;

/// Errors returned by a credential backend.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The backend is temporarily unavailable.
    #[error("credential backend unavailable: {0}")]
    Unavailable(String),
    /// The supplied revision is stale.
    #[error("credential revision conflict")]
    Conflict,
    /// The backend is read-only.
    #[error("credential backend is read-only")]
    ReadOnly,
    /// The backend returned invalid data.
    #[error("invalid credential data: {0}")]
    Invalid(String),
    /// I/O failure.
    #[error("credential I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A pluggable credential source.
#[async_trait]
pub trait CredentialBackend: Send + Sync {
    /// Loads the current snapshot. `None` means the backend has no state yet.
    async fn load(&self) -> Result<Option<CredentialSnapshot>, BackendError>;

    /// Saves a snapshot, using `expected_revision` for compare-and-swap.
    ///
    /// Returns the new revision.
    async fn save(
        &self,
        snapshot: &CredentialSnapshot,
        expected_revision: Option<&str>,
    ) -> Result<String, BackendError>;
}
