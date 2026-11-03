//! Platform error types and API error mapping.

use fastcart_infrastructure::InfrastructureError;
use fastcart_kernel::{AppError, KernelError};

/// Errors raised by platform services.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// Persistence failure.
    #[error("database error: {0}")]
    Database(String),
    /// The requested store does not exist.
    #[error("store not found: {0}")]
    StoreNotFound(String),
    /// The store exists but cannot serve traffic in its current state.
    #[error("store is unavailable: {0}")]
    StoreUnavailable(String),
    /// A hostname is already registered to another store.
    #[error("domain already registered: {0}")]
    DomainConflict(String),
    /// A store slug is already in use.
    #[error("store slug already registered: {0}")]
    SlugConflict(String),
    /// A user email is already registered.
    #[error("email already registered: {0}")]
    EmailConflict(String),
    /// Input failed validation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A requested resource does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The operation conflicts with current state.
    #[error("{0}")]
    Conflict(String),
    /// Email or password is incorrect.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// The session or token is expired or revoked.
    #[error("session expired")]
    SessionExpired,
    /// The caller is not authenticated.
    #[error("unauthorized")]
    Unauthorized,
    /// The caller is not permitted to access the store.
    #[error("forbidden")]
    Forbidden,
    /// password hashing failure.
    #[error("password hashing error: {0}")]
    PasswordHash(String),
    /// A kernel value object was invalid.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// Infrastructure failure.
    #[error(transparent)]
    Infrastructure(#[from] InfrastructureError),
}

impl From<sqlx::Error> for PlatformError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<std::io::Error> for PlatformError {
    fn from(error: std::io::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<PlatformError> for AppError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::Database(message) => Self::Database(message),
            PlatformError::StoreNotFound(message) => Self::not_found("store", message),
            PlatformError::StoreUnavailable(message) => Self::NotFound(message),
            PlatformError::DomainConflict(message) => Self::Conflict(message),
            PlatformError::SlugConflict(message) => Self::Conflict(message),
            PlatformError::EmailConflict(message) => Self::Conflict(message),
            PlatformError::InvalidInput(message) => Self::Validation(message),
            PlatformError::NotFound(message) => Self::NotFound(message),
            PlatformError::Conflict(message) => Self::Conflict(message),
            PlatformError::InvalidCredentials => Self::Unauthorized,
            PlatformError::SessionExpired => Self::Unauthorized,
            PlatformError::Unauthorized => Self::Unauthorized,
            PlatformError::Forbidden => Self::Forbidden,
            PlatformError::PasswordHash(message) => Self::Internal(message),
            PlatformError::Kernel(error) => Self::Validation(error.to_string()),
            PlatformError::Infrastructure(error) => error.into(),
        }
    }
}

/// Convenience result type for the platform crate.
pub type PlatformResult<T> = std::result::Result<T, PlatformError>;

#[cfg(test)]
mod tests {
    use fastcart_kernel::AppError;

    use super::PlatformError;

    #[test]
    fn platform_errors_map_to_stable_api_errors() {
        let mapped: AppError = PlatformError::InvalidCredentials.into();
        assert_eq!(mapped.code(), "UNAUTHORIZED");
        let mapped: AppError = PlatformError::DomainConflict("x".into()).into();
        assert_eq!(mapped.code(), "CONFLICT");
        let mapped: AppError = PlatformError::StoreNotFound("x".into()).into();
        assert_eq!(mapped.code(), "NOT_FOUND");
    }
}
