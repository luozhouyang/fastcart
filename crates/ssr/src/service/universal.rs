//! `UniversalStorefront` implementation for [`SsrService`].

use super::serving::{
    buffer_body, cache_key, cache_ttl, error_response, into_response, serve_asset,
};
use super::*;

#[async_trait::async_trait]
impl UniversalStorefront for SsrService {
    async fn invalidate_cache(&self, store_id: &str) {
        self.invalidate_store_cache(store_id).await;
    }

    async fn render(
        &self,
        request: axum::http::Request<Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        let Some(deployment) = self.deployment(store.store_id.as_str()).await else {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": "SSR_DEPLOYMENT_NOT_FOUND" })),
            )
                .into_response();
        };
        let context = SsrRequestContext {
            store_id: store.store_id.to_string(),
            deployment_id: deployment.deployment_id.clone(),
            request_id: request_id.unwrap_or_else(|| "ssr".to_owned()),
            locale: Some(store.locale),
            currency: Some(store.currency.to_string()),
            market_id: None,
            scheme,
            preview: false,
            internal_token: self
                .credentials
                .token_for_store(store.store_id.as_str(), false),
        };
        self.metrics.record_ssr_request();
        let cache_key = cache_key(&request, store.store_id.as_str());
        if let Some(key) = &cache_key {
            if let Some(response) = self.cached_response(key).await {
                self.metrics.record_ssr_cache_hit();
                return response;
            }
            self.metrics.record_ssr_cache_miss();
        }

        let started = std::time::Instant::now();
        let result = deployment
            .gateway
            .forward_streaming(request, &context)
            .await;
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(mut response) => {
                tracing::debug!(
                    store_id = context.store_id,
                    deployment_id = context.deployment_id,
                    status = %response.status,
                    elapsed_ms,
                    "universal SSR render"
                );
                if response.status == StatusCode::OK
                    && let Some(key) = cache_key
                    && let Some(ttl) = cache_ttl(&response.headers)
                    && let Some(body) = buffer_body(std::mem::replace(
                        &mut response.body,
                        SsrBody::Full(bytes::Bytes::new()),
                    ))
                    .await
                {
                    let cached = CachedResponse {
                        status: response.status,
                        headers: response.headers.clone(),
                        body: body.clone(),
                        stored: Instant::now(),
                        ttl,
                    };
                    let mut cache = self.response_cache.write().await;
                    if cache.len() >= MAX_CACHE_ENTRIES {
                        cache.clear();
                    }
                    cache.insert(key, cached);
                    response.body = SsrBody::Full(body);
                    response.headers.insert(
                        "x-fastcart-cache",
                        axum::http::HeaderValue::from_static("MISS"),
                    );
                }
                into_response(response)
            }
            Err(error) => {
                self.metrics.record_ssr_error();
                tracing::warn!(
                    store_id = context.store_id,
                    deployment_id = context.deployment_id,
                    error = %error,
                    elapsed_ms,
                    "universal SSR render failed"
                );
                error_response(&error)
            }
        }
    }

    async fn serve_asset(
        &self,
        store_id: &str,
        path: String,
        headers: axum::http::HeaderMap,
    ) -> Response {
        let Some(deployment) = self.deployment(store_id).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let response = serve_asset(&deployment.asset_root, &path, &headers).await;
        if response.status() != StatusCode::NOT_FOUND {
            return response;
        }
        for root in self.asset_roots.read().await.iter() {
            let response = serve_asset(root, &path, &headers).await;
            if response.status() != StatusCode::NOT_FOUND {
                return response;
            }
        }
        StatusCode::NOT_FOUND.into_response()
    }

    async fn serve_preview_asset(
        &self,
        store_id: &str,
        path: String,
        headers: axum::http::HeaderMap,
    ) -> Response {
        let Some(deployment) = self.preview_deployment(store_id).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let response = serve_asset(&deployment.asset_root, &path, &headers).await;
        if response.status() != StatusCode::NOT_FOUND {
            return response;
        }
        for root in self.preview_asset_roots.read().await.iter() {
            let response = serve_asset(root, &path, &headers).await;
            if response.status() != StatusCode::NOT_FOUND {
                return response;
            }
        }
        StatusCode::NOT_FOUND.into_response()
    }

    async fn render_preview(
        &self,
        request: axum::http::Request<Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        let Some(deployment) = self.preview_deployment(store.store_id.as_str()).await else {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({ "error": "SSR_PREVIEW_NOT_FOUND" })),
            )
                .into_response();
        };
        self.metrics.record_ssr_request();
        let context = SsrRequestContext {
            store_id: store.store_id.to_string(),
            deployment_id: deployment.deployment_id.clone(),
            request_id: request_id.unwrap_or_else(|| "ssr-preview".to_owned()),
            locale: Some(store.locale),
            currency: Some(store.currency.to_string()),
            market_id: None,
            scheme,
            preview: true,
            internal_token: self
                .credentials
                .token_for_store(store.store_id.as_str(), true),
        };
        match deployment
            .gateway
            .forward_streaming(request, &context)
            .await
        {
            Ok(response) => into_response(response),
            Err(error) => {
                self.metrics.record_ssr_error();
                error_response(&error)
            }
        }
    }

    async fn has_preview(&self, store_id: &str) -> bool {
        self.previews.read().await.contains_key(store_id)
    }

    async fn activate_preview(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String> {
        self.activate_preview(store_id, deployment_id, Path::new(&artifact_path))
            .await
            .map_err(|error| error.to_string())
    }

    async fn deactivate_preview(&self, store_id: &str) -> Result<(), String> {
        self.deactivate_preview(store_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String> {
        self.activate(store_id, deployment_id, Path::new(&artifact_path))
            .await
            .map_err(|error| error.to_string())
    }

    async fn rollback(&self, store_id: &str) -> Result<(), String> {
        self.rollback(store_id)
            .await
            .map_err(|error| error.to_string())
    }
}
