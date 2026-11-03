//! Kernel-level value errors and the shared application error model.

/// Errors raised while constructing or combining kernel value objects.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// A currency code is not a three-letter ISO-style code.
    #[error("invalid currency code: {0}")]
    InvalidCurrency(String),

    /// Two monetary values use different currencies.
    #[error("currency mismatch: {left} and {right}")]
    CurrencyMismatch {
        /// Currency of the left operand.
        left: String,
        /// Currency of the right operand.
        right: String,
    },

    /// Generic validation failure.
    #[error("{0}")]
    Validation(String),
}

/// Shared application error categories.
///
/// Presentation layers map these variants to stable API error codes. Database
/// and provider details must not be forwarded to clients verbatim.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Input failed validation.
    #[error("{0}")]
    Validation(String),
    /// A requested resource does not exist.
    #[error("{0}")]
    NotFound(String),
    /// The request conflicts with current state.
    #[error("{0}")]
    Conflict(String),
    /// Authentication is missing or invalid.
    #[error("unauthorized")]
    Unauthorized,
    /// The actor is not allowed to perform the operation.
    #[error("forbidden")]
    Forbidden,
    /// The caller exceeded a rate limit.
    #[error("rate limited")]
    RateLimited,
    /// Persistence layer failure.
    #[error("database error: {0}")]
    Database(String),
    /// External provider failure.
    #[error("external service error: {0}")]
    ExternalService(String),
    /// Unexpected internal failure.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "VALIDATION_ERROR",
            Self::NotFound(_) => "NOT_FOUND",
            Self::Conflict(_) => "CONFLICT",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::Forbidden => "FORBIDDEN",
            Self::RateLimited => "RATE_LIMITED",
            Self::Database(_) => "DATABASE_ERROR",
            Self::ExternalService(_) => "EXTERNAL_SERVICE_ERROR",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }
}
