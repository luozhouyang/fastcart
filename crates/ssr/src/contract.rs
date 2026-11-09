//! The HTTP-facing contract between the commerce server and the SSR service.

use axum::http::HeaderMap;
use axum::response::IntoResponse;
use fastcart_kernel::StoreContext;

/// Process-wide storefront SSR contract.
///
/// The API layer only depends on this trait; [`crate::SsrService`] is the
/// production implementation and tests can provide their own.
#[async_trait::async_trait]
pub trait UniversalStorefront: Send + Sync {
    /// Forwards a storefront request to the configured artifact runtime.
    async fn render(
        &self,
        request: axum::http::Request<axum::body::Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> axum::response::Response;

    /// Serves one client asset from the active artifact.
    async fn serve_asset(
        &self,
        store_id: &str,
        path: String,
        headers: HeaderMap,
    ) -> axum::response::Response;

    /// Starts and activates a new deployment for one store.
    async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String>;

    /// Restores the previous deployment for one store.
    async fn rollback(&self, store_id: &str) -> Result<(), String>;

    /// Invalidates cached SSR responses for one store.
    async fn invalidate_cache(&self, _store_id: &str) {}

    /// Renders through the store's preview deployment.
    async fn render_preview(
        &self,
        _request: axum::http::Request<axum::body::Body>,
        _store: StoreContext,
        _request_id: Option<String>,
        _scheme: String,
    ) -> axum::response::Response {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }

    /// Returns whether the store has a preview deployment.
    async fn has_preview(&self, _store_id: &str) -> bool {
        false
    }

    /// Serves one client asset from the store's preview artifact.
    async fn serve_preview_asset(
        &self,
        _store_id: &str,
        _path: String,
        _headers: HeaderMap,
    ) -> axum::response::Response {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }

    /// Starts and activates a preview deployment.
    async fn activate_preview(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Err("preview deployments are not supported".to_owned())
    }

    /// Stops the store's preview deployment.
    async fn deactivate_preview(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }
}
