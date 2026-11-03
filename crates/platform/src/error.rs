//! Platform error types.

use fastcart_kernel::KernelError;

/// Errors raised by platform services.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// Persistence failure.
    #[error("database error: {0}")]
    Database(String),
    /// The requested store does not exist.
    #[error("store not found: {0}")]
    StoreNotFound(String),
    /// A hostname is already registered to another store.
    #[error("domain already registered: {0}")]
    DomainConflict(String),
    /// Input failed validation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A kernel value object was invalid.
    #[error(transparent)]
    Kernel(#[from] KernelError),
}
