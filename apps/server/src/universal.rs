//! Universal workerd storefront runtime wiring.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use fastcart_api::state::UniversalStorefront;
use fastcart_kernel::StoreContext;
use fastcart_ssr::artifact::{ArtifactLimits, ArtifactStore};
use fastcart_ssr::{
    RuntimeGroupId, RuntimeManager, RuntimeManagerConfig, SsrBody, SsrError, SsrGateway,
    SsrRequestContext, SsrResponse,
};
use serde_json::json;
use tokio::sync::Mutex;

use crate::config::SsrConfig;

/// One active universal storefront runtime.
pub struct UniversalSsr {
    manager: Mutex<RuntimeManager>,
    gateway: SsrGateway,
    group: RuntimeGroupId,
    deployment_id: String,
    asset_root: std::path::PathBuf,
}

impl UniversalSsr {
    /// Imports the configured artifact and starts its workerd group.
    pub async fn initialize(config: &SsrConfig) -> Result<Option<Arc<Self>>> {
        if !config.enabled {
            return Ok(None);
        }
        let store = ArtifactStore::new(config.runtime_dir.join("artifacts"));
        let limits = ArtifactLimits {
            max_bytes: config.artifact_max_bytes,
            max_files: config.artifact_max_files,
        };
        let artifact = store
            .import(&config.artifact_path, limits)
            .context("failed to import the SSR artifact")?;
        let asset_root = artifact.root().join(&artifact.manifest().assets.directory);
        let mut manager_config =
            RuntimeManagerConfig::new(config.runtime_dir.join("runtime"), config.binary.clone());
        manager_config.startup_timeout = Duration::from_millis(config.startup_timeout_ms);
        manager_config.shutdown_timeout = Duration::from_millis(config.shutdown_timeout_ms);
        manager_config.request_timeout = Duration::from_millis(config.request_timeout_ms);
        manager_config.restart_backoff = Duration::from_millis(config.restart_backoff_ms);
        manager_config.max_restarts = config.max_restarts;
        let mut manager = RuntimeManager::new(manager_config)
            .context("failed to create the SSR runtime manager")?;
        let group = RuntimeGroupId::new("active").context("invalid SSR runtime group")?;
        let health = manager
            .start_group(group.clone(), artifact, config.listen)
            .await
            .context("failed to start the SSR runtime")?;
        if health.status != fastcart_ssr::RuntimeStatus::Healthy {
            bail!("SSR runtime started but did not become healthy");
        }
        let gateway = manager
            .gateway(&group)
            .context("failed to create the SSR gateway")?
            .with_internal_token(config.internal_token.clone().unwrap_or_default());
        Ok(Some(Arc::new(Self {
            manager: Mutex::new(manager),
            gateway,
            group,
            deployment_id: config.deployment_id.clone(),
            asset_root,
        })))
    }

    /// Stops the runtime group during server shutdown.
    pub async fn shutdown(&self) {
        let mut manager = self.manager.lock().await;
        if let Err(error) = manager.stop_group(&self.group).await {
            tracing::warn!(error = %error, "failed to stop the universal SSR runtime");
        }
    }
}

#[async_trait::async_trait]
impl UniversalStorefront for UniversalSsr {
    async fn render(
        &self,
        request: axum::http::Request<Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        let context = SsrRequestContext {
            store_id: store.store_id.to_string(),
            deployment_id: self.deployment_id.clone(),
            request_id: request_id.unwrap_or_else(|| "ssr".to_owned()),
            locale: Some(store.locale),
            currency: Some(store.currency.to_string()),
            market_id: None,
            scheme,
        };
        match self.gateway.forward_streaming(request, &context).await {
            Ok(response) => into_response(response),
            Err(error) => error_response(&error),
        }
    }

    async fn serve_asset(&self, path: String) -> Response {
        serve_asset(&self.asset_root, &path).await
    }
}

async fn serve_asset(root: &std::path::Path, path: &str) -> Response {
    let mut relative = std::path::PathBuf::new();
    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Normal(segment) => relative.push(segment),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    }
    if relative.as_os_str().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(full) = tokio::fs::canonicalize(root.join(&relative)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !full.starts_with(&root) || !full.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(bytes) = tokio::fs::read(&full).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = content_type(&full);
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .header(
            axum::http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        )
        .body(Body::from(bytes))
        .unwrap_or_else(|error| {
            tracing::error!(error = %error, "failed to build asset response");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

fn content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn into_response(response: SsrResponse) -> Response {
    let mut builder = Response::builder().status(response.status);
    if let Some(headers) = builder.headers_mut() {
        for (name, value) in &response.headers {
            headers.append(name, value.clone());
        }
    }
    let body = match response.body {
        SsrBody::Full(bytes) => Body::from(bytes),
        SsrBody::Stream(stream) => Body::from_stream(stream),
    };
    builder.body(body).unwrap_or_else(|error| {
        tracing::error!(error = %error, "failed to build SSR response");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

fn error_response(error: &SsrError) -> Response {
    let (status, code) = match error {
        SsrError::Health(_)
        | SsrError::NotRunning
        | SsrError::StartupTimeout(_)
        | SsrError::EarlyExit(_)
        | SsrError::Start(_) => (StatusCode::SERVICE_UNAVAILABLE, "SSR_RUNTIME_UNAVAILABLE"),
        SsrError::InvalidRequest(_)
        | SsrError::InvalidRuntimeConfig(_)
        | SsrError::UnknownGroup(_)
        | SsrError::AlreadyRunning(_)
        | SsrError::RestartLimit(_) => (StatusCode::BAD_GATEWAY, "SSR_RUNTIME_ERROR"),
        _ => (StatusCode::BAD_GATEWAY, "SSR_REQUEST_FAILED"),
    };
    tracing::warn!(error = %error, status = %status, "universal SSR request failed");
    (status, axum::Json(json!({ "error": code }))).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use fastcart_kernel::{CurrencyCode, StoreContext, StoreId};
    use fastcart_ssr::artifact::{
        ArtifactManifest, AssetMetadata, BuildMetadata, EntryMetadata, FrameworkMetadata,
        RenderingMetadata, RuntimeMetadata,
    };
    use std::path::PathBuf;

    use super::*;

    fn workerd_binary() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("FASTCART_WORKERD_BIN").map(PathBuf::from) {
            return path.is_file().then_some(path);
        }
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../experiments/workerd/bin/workerd");
        path.is_file().then_some(path)
    }

    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port");
        listener.local_addr().expect("addr").port()
    }

    fn manifest() -> ArtifactManifest {
        ArtifactManifest {
            schema_version: "1".to_owned(),
            artifact_id: "server_fixture".to_owned(),
            framework: FrameworkMetadata {
                name: "fixture".to_owned(),
                version: "1.0.0".to_owned(),
            },
            runtime: RuntimeMetadata {
                engine: "workerd".to_owned(),
                compatibility_date: "2026-11-01".to_owned(),
                compatibility_flags: vec![],
                required_capabilities: vec!["fetch".to_owned()],
            },
            entry: EntryMetadata {
                entry_type: "worker".to_owned(),
                path: "worker/index.js".to_owned(),
                export: "default".to_owned(),
            },
            assets: AssetMetadata {
                directory: "client".to_owned(),
                base_path: "/_fastcart/assets/".to_owned(),
            },
            rendering: RenderingMetadata {
                mode: "hybrid".to_owned(),
                streaming: true,
            },
            build: BuildMetadata {
                build_id: "build_server".to_owned(),
                created_at: "2026-11-01T10:00:00Z".to_owned(),
            },
        }
    }

    fn artifact(root: &std::path::Path) -> std::path::PathBuf {
        let source = root.join("source");
        std::fs::create_dir_all(source.join("worker")).expect("worker");
        std::fs::create_dir_all(source.join("client")).expect("client");
        std::fs::write(
            source.join("worker/index.js"),
            r#"
export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/__fastcart/health") {
      return Response.json({ status: "ok" });
    }
    return Response.json({
      path: url.pathname,
      store: request.headers.get("x-fastcart-store-id"),
      deployment: request.headers.get("x-fastcart-deployment-id"),
      requestId: request.headers.get("x-fastcart-request-id"),
      proto: request.headers.get("x-forwarded-proto"),
      token: request.headers.get("x-fastcart-internal-token"),
    });
  },
};
"#,
        )
        .expect("worker");
        std::fs::write(source.join("client/app.js"), "console.log('asset');").expect("asset");
        std::fs::write(
            source.join("fastcart-ssr.json"),
            serde_json::to_vec_pretty(&manifest()).expect("manifest"),
        )
        .expect("manifest");
        source
    }

    #[tokio::test]
    async fn initialize_render_and_shutdown() {
        let Some(binary) = workerd_binary() else {
            eprintln!("skipping: no workerd binary");
            return;
        };
        let root = tempfile::tempdir().expect("root");
        let source = artifact(root.path());
        let config = SsrConfig {
            enabled: true,
            binary,
            artifact_path: source,
            runtime_dir: root.path().join("runtime"),
            listen: format!("127.0.0.1:{}", free_port())
                .parse()
                .expect("listen"),
            deployment_id: "deploy_server".to_owned(),
            internal_token: Some("server-secret".to_owned()),
            request_timeout_ms: 10_000,
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 5_000,
            restart_backoff_ms: 10,
            max_restarts: 1,
            artifact_max_bytes: 10 * 1024 * 1024,
            artifact_max_files: 100,
        };
        let runtime = UniversalSsr::initialize(&config)
            .await
            .expect("initialize")
            .expect("enabled");
        let request = Request::builder()
            .uri("/context")
            .body(Body::empty())
            .expect("request");
        let store = StoreContext {
            store_id: StoreId::from_string("store_01".to_owned()),
            domain: "shop.example".to_owned(),
            locale: "en-US".to_owned(),
            currency: CurrencyCode::new("USD").expect("currency"),
            timezone: "UTC".to_owned(),
        };
        let response = runtime
            .render(
                request,
                store,
                Some("req_server".to_owned()),
                "https".to_owned(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["store"], "store_01");
        assert_eq!(value["deployment"], "deploy_server");
        assert_eq!(value["requestId"], "req_server");
        assert_eq!(value["proto"], "https");
        assert_eq!(value["token"], "server-secret");

        let asset = runtime.serve_asset("app.js".to_owned()).await;
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(
            asset
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            asset
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "public, max-age=31536000, immutable"
        );
        let asset_body = axum::body::to_bytes(asset.into_body(), 1024)
            .await
            .expect("asset body");
        assert_eq!(&asset_body[..], b"console.log('asset');");

        let traversal = runtime.serve_asset("../fastcart-ssr.json".to_owned()).await;
        assert_eq!(traversal.status(), StatusCode::NOT_FOUND);
        let missing = runtime.serve_asset("missing.js".to_owned()).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        runtime.shutdown().await;
    }
}
