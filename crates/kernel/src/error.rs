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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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
    /// The caller is not authenticated.
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
    /// The feature is not implemented yet.
    #[error("not implemented: {0}")]
    NotImplemented(String),
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
            Self::NotImplemented(_) => "NOT_IMPLEMENTED",
        }
    }

    /// Creates a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    /// Creates a not-found error for a named resource.
    pub fn not_found(resource: &str, id: impl std::fmt::Display) -> Self {
        Self::NotFound(format!("{resource} not found: {id}"))
    }

    /// Creates a conflict error.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    /// Creates a database error.
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database(message.into())
    }

    /// Creates an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// Returns true when the error represents a missing resource.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl From<KernelError> for AppError {
    fn from(error: KernelError) -> Self {
        Self::Validation(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(AppError::Validation("x".into()).code(), "VALIDATION_ERROR");
        assert_eq!(AppError::Unauthorized.code(), "UNAUTHORIZED");
        assert_eq!(AppError::Forbidden.code(), "FORBIDDEN");
        assert_eq!(AppError::not_found("product", "prod_1").code(), "NOT_FOUND");
    }

    #[test]
    fn not_found_messages_include_the_identifier() {
        let error = AppError::not_found("product", "prod_1");
        assert!(error.to_string().contains("prod_1"));
    }
}
