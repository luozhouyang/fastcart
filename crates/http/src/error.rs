//! Stable API error envelope and HTTP status mapping.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use fastcart_kernel::AppError;
use fastcart_platform::PlatformError;
use serde_json::json;

tokio::task_local! {
    /// Correlation identifier available while a request is being handled.
    pub static REQUEST_ID: String;
}

/// Result type used by HTTP handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// An application error rendered as the platform error envelope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    error: AppError,
}

impl ApiError {
    /// Creates an API error from an application error.
    #[must_use]
    pub fn new(error: AppError) -> Self {
        Self { error }
    }

    /// Returns the underlying application error.
    #[must_use]
    pub fn inner(&self) -> &AppError {
        &self.error
    }

    /// Validation failure.
    #[must_use]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(AppError::Validation(message.into()))
    }

    /// Missing resource.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(AppError::NotFound(message.into()))
    }

    /// Stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.error.code()
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self::new(error)
    }
}

impl From<PlatformError> for ApiError {
    fn from(error: PlatformError) -> Self {
        Self::new(error.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = status_for(&self.error);
        let public_message = match &self.error {
            AppError::Database(_) | AppError::Internal(_) => {
                tracing::error!(error = %self.error, "request failed with an internal error");
                "an internal error occurred".to_owned()
            }
            AppError::ExternalService(_) => {
                tracing::warn!(error = %self.error, "external service failure");
                "an external service is unavailable".to_owned()
            }
            other => other.to_string(),
        };
        let request_id = REQUEST_ID.try_with(Clone::clone).ok();
        let body = Json(json!({
            "error": {
                "code": self.error.code(),
                "message": public_message,
                "request_id": request_id,
            }
        }));
        (status, body).into_response()
    }
}

fn status_for(error: &AppError) -> StatusCode {
    match error {
        AppError::Validation(_) => StatusCode::BAD_REQUEST,
        AppError::NotFound(_) => StatusCode::NOT_FOUND,
        AppError::Conflict(_) => StatusCode::CONFLICT,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        AppError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        AppError::Database(_) | AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        AppError::ExternalService(_) => StatusCode::BAD_GATEWAY,
        AppError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{ApiError, ApiResult};
    use fastcart_kernel::AppError;

    async fn failing() -> ApiResult<()> {
        Err(ApiError::new(AppError::not_found("product", "prod_1")))
    }

    async fn internal() -> ApiResult<()> {
        Err(ApiError::new(AppError::Database("secret details".into())))
    }

    fn app() -> Router {
        Router::new()
            .route("/missing", get(failing))
            .route("/internal", get(internal))
    }

    #[tokio::test]
    async fn renders_the_error_envelope() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "NOT_FOUND");
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("prod_1")
        );
    }

    #[tokio::test]
    async fn hides_internal_details() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/internal")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "DATABASE_ERROR");
        assert!(
            !value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("secret")
        );
    }
}
