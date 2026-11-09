//! Universal workerd storefront runtime wiring.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};

use crate::config::SsrConfig;

/// One active storefront deployment.
#[derive(Clone)]
struct Deployment {
    group: RuntimeGroupId,
    gateway: SsrGateway,
    deployment_id: String,
    asset_root: PathBuf,
    artifact_path: PathBuf,
}

/// A deployment that can be restored by a rollback.
#[derive(Clone)]
struct DeploymentMeta {
    deployment_id: String,
    artifact_path: PathBuf,
}

/// One buffered SSR response eligible for route-level caching.
#[derive(Clone)]
struct CachedResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: bytes::Bytes,
    stored: Instant,
    ttl: Duration,
}

const MAX_CACHE_ENTRY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 1_000;

/// Activation state written atomically after a successful swap.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedDeployment {
    store_id: String,
    deployment_id: String,
    artifact_path: PathBuf,
    #[serde(default)]
    preview: bool,
}

/// Universal storefront runtime registry.
pub struct UniversalSsr {
    manager: Arc<Mutex<RuntimeManager>>,
    deployments: RwLock<HashMap<String, Arc<Deployment>>>,
    previews: RwLock<HashMap<String, Arc<Deployment>>>,
    history: RwLock<HashMap<String, Vec<DeploymentMeta>>>,
    asset_roots: RwLock<Vec<PathBuf>>,
    response_cache: RwLock<HashMap<String, CachedResponse>>,
    default: Option<Arc<Deployment>>,
    artifacts: ArtifactStore,
    artifact_root: Option<PathBuf>,
    state_path: PathBuf,
    config: SsrConfig,
    counter: AtomicU64,
}

struct DeploymentSpec<'a> {
    store_id: Option<&'a str>,
    deployment_id: &'a str,
    artifact_path: &'a Path,
    listen: SocketAddr,
    preview: bool,
}

impl UniversalSsr {
    /// Imports configured artifacts and starts one workerd group per deployment.
    pub async fn initialize(config: &SsrConfig) -> Result<Option<Arc<Self>>> {
        if !config.enabled {
            return Ok(None);
        }
        let artifacts = ArtifactStore::new(config.runtime_dir.join("artifacts"));
        let limits = ArtifactLimits {
            max_bytes: config.artifact_max_bytes,
            max_files: config.artifact_max_files,
        };
        let mut manager_config =
            RuntimeManagerConfig::new(config.runtime_dir.join("runtime"), config.binary.clone());
        manager_config.startup_timeout = Duration::from_millis(config.startup_timeout_ms);
        manager_config.shutdown_timeout = Duration::from_millis(config.shutdown_timeout_ms);
        manager_config.request_timeout = Duration::from_millis(config.request_timeout_ms);
        manager_config.restart_backoff = Duration::from_millis(config.restart_backoff_ms);
        manager_config.max_restarts = config.max_restarts;
        let mut manager = RuntimeManager::new(manager_config)
            .context("failed to create the SSR runtime manager")?;
        let state_path = config.runtime_dir.join("deployment-state.json");
        let persisted_config = if config.deployments.is_empty() {
            load_persisted(&state_path).await.map(|entries| {
                let mut persisted = config.clone();
                persisted.deployments = entries
                    .into_iter()
                    .map(|entry| crate::config::SsrDeploymentConfig {
                        store_id: entry.store_id,
                        deployment_id: entry.deployment_id,
                        artifact_path: entry.artifact_path,
                        listen: None,
                        preview: entry.preview,
                    })
                    .collect();
                persisted
            })
        } else {
            None
        };
        let specs = deployment_specs(persisted_config.as_ref().unwrap_or(config));
        let mut deployments = HashMap::new();
        let mut previews = HashMap::new();
        let mut default = None;
        let mut started = Vec::new();
        for spec in specs {
            let group = RuntimeGroupId::new(spec.store_id.unwrap_or("default"))
                .context("invalid SSR runtime group")?;
            let result = start_deployment(
                &mut manager,
                &artifacts,
                limits,
                config,
                &spec,
                group.clone(),
            )
            .await;
            match result {
                Ok(deployment) => {
                    started.push(group);
                    if spec.store_id.is_some() {
                        let deployment = Arc::new(deployment);
                        if spec.preview {
                            previews
                                .insert(spec.store_id.unwrap_or_default().to_owned(), deployment);
                        } else {
                            deployments
                                .insert(spec.store_id.unwrap_or_default().to_owned(), deployment);
                        }
                    } else {
                        default = Some(Arc::new(deployment));
                    }
                }
                Err(error) => {
                    for group in &started {
                        if let Err(stop_error) = manager.stop_group(group).await {
                            tracing::warn!(error = %stop_error, group = %group, "failed to stop partially started SSR group");
                        }
                    }
                    return Err(error);
                }
            }
        }
        let mut asset_roots: Vec<PathBuf> = deployments
            .values()
            .chain(previews.values())
            .map(|deployment| deployment.asset_root.clone())
            .collect();
        if let Some(default) = &default {
            asset_roots.push(default.asset_root.clone());
        }
        Ok(Some(Arc::new(Self {
            manager: Arc::new(Mutex::new(manager)),
            deployments: RwLock::new(deployments),
            previews: RwLock::new(previews),
            history: RwLock::new(HashMap::new()),
            asset_roots: RwLock::new(asset_roots),
            response_cache: RwLock::new(HashMap::new()),
            default,
            artifacts,
            artifact_root: config.artifact_root.clone(),
            state_path,
            config: config.clone(),
            counter: AtomicU64::new(0),
        })))
    }

    /// Starts a new artifact for one store and drains the replaced deployment.
    pub async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
    ) -> Result<()> {
        self.replace(store_id, deployment_id, artifact_path, true)
            .await
    }

    /// Restores the deployment replaced by the last activation.
    pub async fn rollback(&self, store_id: &str) -> Result<()> {
        let previous = {
            let mut history = self.history.write().await;
            history.get_mut(store_id).and_then(std::vec::Vec::pop)
        };
        let Some(previous) = previous else {
            bail!("no previous deployment for {store_id}");
        };
        self.replace(
            store_id,
            &previous.deployment_id,
            &previous.artifact_path,
            true,
        )
        .await
    }

    async fn replace(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
        record_history: bool,
    ) -> Result<()> {
        validate_identifier("store_id", store_id)?;
        validate_identifier("deployment_id", deployment_id)?;
        let artifact_path = self.resolve_artifact_path(artifact_path).await?;
        let listen = free_loopback_port()?;
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        let group = RuntimeGroupId::new(format!("active_{sequence}"))
            .context("invalid activation group")?;
        let spec = DeploymentSpec {
            store_id: Some(store_id),
            deployment_id,
            artifact_path: &artifact_path,
            listen,
            preview: false,
        };
        let limits = ArtifactLimits {
            max_bytes: self.config.artifact_max_bytes,
            max_files: self.config.artifact_max_files,
        };
        let mut manager = self.manager.lock().await;
        let deployment = start_deployment(
            &mut manager,
            &self.artifacts,
            limits,
            &self.config,
            &spec,
            group,
        )
        .await?;
        drop(manager);

        let deployment = Arc::new(deployment);
        let old = self
            .deployments
            .write()
            .await
            .insert(store_id.to_owned(), deployment);
        let previous = old.or_else(|| self.default.clone());
        if record_history && let Some(previous) = &previous {
            let mut history = self.history.write().await;
            let entries = history.entry(store_id.to_owned()).or_default();
            entries.push(DeploymentMeta {
                deployment_id: previous.deployment_id.clone(),
                artifact_path: previous.artifact_path.clone(),
            });
            if entries.len() > 3 {
                entries.remove(0);
            }
        }
        self.invalidate_store_cache(store_id).await;
        self.persist_state().await;
        if let Some(previous) = previous {
            tracing::info!(store_id, previous = %previous.deployment_id, "draining replaced SSR deployment");
            let mut roots = self.asset_roots.write().await;
            if !roots.iter().any(|root| root == &previous.asset_root) {
                roots.push(previous.asset_root.clone());
            }
            drop(roots);
            if self
                .default
                .as_ref()
                .is_none_or(|default| default.group.as_str() != previous.group.as_str())
            {
                self.schedule_drain(previous.group.clone());
            }
        }
        Ok(())
    }

    async fn persist_state(&self) {
        let mut entries: Vec<PersistedDeployment> = self
            .deployments
            .read()
            .await
            .iter()
            .map(|(store_id, deployment)| PersistedDeployment {
                store_id: store_id.clone(),
                deployment_id: deployment.deployment_id.clone(),
                artifact_path: deployment.artifact_path.clone(),
                preview: false,
            })
            .collect();
        entries.extend(
            self.previews
                .read()
                .await
                .iter()
                .map(|(store_id, deployment)| PersistedDeployment {
                    store_id: store_id.clone(),
                    deployment_id: deployment.deployment_id.clone(),
                    artifact_path: deployment.artifact_path.clone(),
                    preview: true,
                }),
        );
        if entries.is_empty() {
            return;
        }
        let Ok(serialized) = serde_json::to_vec_pretty(&entries) else {
            tracing::warn!("failed to serialize SSR deployment state");
            return;
        };
        let staging = self.state_path.with_extension("json.tmp");
        if let Some(parent) = self.state_path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            tracing::warn!(error = %error, "failed to create SSR state directory");
            return;
        }
        if let Err(error) = tokio::fs::write(&staging, serialized).await {
            tracing::warn!(error = %error, "failed to write SSR deployment state");
            return;
        }
        if let Err(error) = tokio::fs::rename(&staging, &self.state_path).await {
            tracing::warn!(error = %error, "failed to commit SSR deployment state");
        }
    }

    async fn cached_response(&self, key: &str) -> Option<Response> {
        let entry = {
            let mut cache = self.response_cache.write().await;
            let entry = cache.get(key).cloned()?;
            if entry.stored.elapsed() > entry.ttl {
                cache.remove(key);
                return None;
            }
            entry
        };
        let mut builder = Response::builder().status(entry.status);
        if let Some(headers) = builder.headers_mut() {
            for (name, value) in &entry.headers {
                headers.append(name, value.clone());
            }
            headers.insert(
                "x-fastcart-cache",
                axum::http::HeaderValue::from_static("HIT"),
            );
            headers.insert(
                axum::http::header::AGE,
                axum::http::HeaderValue::from_str(&entry.stored.elapsed().as_secs().to_string())
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("0")),
            );
        }
        builder.body(Body::from(entry.body)).ok()
    }

    /// Invalidates cached SSR responses for one store.
    pub async fn invalidate_store_cache(&self, store_id: &str) {
        let prefix = format!("{store_id}:");
        self.response_cache
            .write()
            .await
            .retain(|key, _| !key.starts_with(&prefix));
    }

    async fn resolve_artifact_path(&self, path: &Path) -> Result<PathBuf> {
        let Some(root) = &self.artifact_root else {
            return Ok(path.to_owned());
        };
        let root = tokio::fs::canonicalize(root)
            .await
            .with_context(|| format!("failed to resolve artifact root {}", root.display()))?;
        let candidate = tokio::fs::canonicalize(path)
            .await
            .with_context(|| format!("failed to resolve artifact path {}", path.display()))?;
        if !candidate.starts_with(&root) {
            bail!("artifact path must stay under {}", root.display());
        }
        Ok(candidate)
    }

    fn schedule_drain(&self, group: RuntimeGroupId) {
        let manager = Arc::clone(&self.manager);
        let delay = Duration::from_millis(self.config.drain_timeout_ms.max(1));
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Err(error) = manager.lock().await.stop_group(&group).await {
                tracing::warn!(error = %error, group = %group, "failed to drain SSR group");
            }
        });
    }

    /// Starts a preview deployment without touching production.
    pub async fn activate_preview(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: &Path,
    ) -> Result<()> {
        validate_identifier("store_id", store_id)?;
        validate_identifier("deployment_id", deployment_id)?;
        let artifact_path = self.resolve_artifact_path(artifact_path).await?;
        let listen = free_loopback_port()?;
        let sequence = self.counter.fetch_add(1, Ordering::Relaxed);
        let group =
            RuntimeGroupId::new(format!("preview_{sequence}")).context("invalid preview group")?;
        let spec = DeploymentSpec {
            store_id: Some(store_id),
            deployment_id,
            artifact_path: &artifact_path,
            listen,
            preview: true,
        };
        let limits = ArtifactLimits {
            max_bytes: self.config.artifact_max_bytes,
            max_files: self.config.artifact_max_files,
        };
        let mut manager = self.manager.lock().await;
        tracing::info!(store_id, deployment_id, path = %artifact_path.display(), "activating SSR preview");
        let deployment = start_deployment(
            &mut manager,
            &self.artifacts,
            limits,
            &self.config,
            &spec,
            group,
        )
        .await?;
        drop(manager);
        let deployment = Arc::new(deployment);
        let old = self
            .previews
            .write()
            .await
            .insert(store_id.to_owned(), deployment);
        if let Some(old) = old {
            let mut roots = self.asset_roots.write().await;
            if !roots.iter().any(|root| root == &old.asset_root) {
                roots.push(old.asset_root.clone());
            }
            drop(roots);
            self.schedule_drain(old.group.clone());
        }
        self.persist_state().await;
        Ok(())
    }

    /// Stops the store's preview deployment.
    pub async fn deactivate_preview(&self, store_id: &str) -> Result<()> {
        let Some(old) = self.previews.write().await.remove(store_id) else {
            return Ok(());
        };
        let mut roots = self.asset_roots.write().await;
        if !roots.iter().any(|root| root == &old.asset_root) {
            roots.push(old.asset_root.clone());
        }
        drop(roots);
        self.schedule_drain(old.group.clone());
        self.persist_state().await;
        Ok(())
    }

    async fn preview_deployment(&self, store_id: &str) -> Option<Arc<Deployment>> {
        self.previews.read().await.get(store_id).cloned()
    }

    /// Stops every runtime group during server shutdown.
    pub async fn shutdown(&self) {
        let mut manager = self.manager.lock().await;
        let mut groups: Vec<RuntimeGroupId> = self
            .deployments
            .read()
            .await
            .values()
            .chain(self.previews.read().await.values())
            .map(|deployment| deployment.group.clone())
            .collect();
        if let Some(default) = &self.default {
            groups.push(default.group.clone());
        }
        for group in groups {
            if let Err(error) = manager.stop_group(&group).await {
                tracing::warn!(error = %error, group = %group, "failed to stop the universal SSR runtime");
            }
        }
    }

    async fn deployment(&self, store_id: &str) -> Option<Arc<Deployment>> {
        self.deployments
            .read()
            .await
            .get(store_id)
            .cloned()
            .or_else(|| self.default.clone())
    }
}

async fn load_persisted(path: &Path) -> Option<Vec<PersistedDeployment>> {
    let bytes = tokio::fs::read(path).await.ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(entries) => {
            let entries: Vec<PersistedDeployment> = entries;
            (!entries.is_empty()).then_some(entries)
        }
        Err(error) => {
            tracing::warn!(error = %error, path = %path.display(), "ignoring invalid SSR deployment state");
            None
        }
    }
}

fn deployment_specs(config: &SsrConfig) -> Vec<DeploymentSpec<'_>> {
    if config.deployments.is_empty() {
        return vec![DeploymentSpec {
            store_id: None,
            deployment_id: &config.deployment_id,
            artifact_path: &config.artifact_path,
            listen: config.listen,
            preview: false,
        }];
    }
    config
        .deployments
        .iter()
        .enumerate()
        .map(|(index, deployment)| {
            let listen = deployment.listen.unwrap_or_else(|| {
                let offset = u16::try_from(index).unwrap_or(u16::MAX);
                SocketAddr::new(
                    config.listen.ip(),
                    config.listen.port().saturating_add(offset),
                )
            });
            DeploymentSpec {
                store_id: Some(&deployment.store_id),
                deployment_id: &deployment.deployment_id,
                artifact_path: &deployment.artifact_path,
                listen,
                preview: deployment.preview,
            }
        })
        .collect()
}

async fn start_deployment(
    manager: &mut RuntimeManager,
    store: &ArtifactStore,
    limits: ArtifactLimits,
    config: &SsrConfig,
    spec: &DeploymentSpec<'_>,
    group: RuntimeGroupId,
) -> Result<Deployment> {
    let artifact = store.import(spec.artifact_path, limits).with_context(|| {
        format!(
            "failed to import the SSR artifact for {}",
            spec.store_id.unwrap_or("default")
        )
    })?;
    let asset_root = artifact.root().join(&artifact.manifest().assets.directory);
    let health = manager
        .start_group(group.clone(), artifact, spec.listen)
        .await
        .with_context(|| {
            format!(
                "failed to start the SSR runtime for {}",
                spec.store_id.unwrap_or("default")
            )
        })?;
    if health.status != fastcart_ssr::RuntimeStatus::Healthy {
        let _ = manager.stop_group(&group).await;
        bail!("SSR runtime started but did not become healthy");
    }
    let gateway = manager
        .gateway(&group)
        .context("failed to create the SSR gateway")?
        .with_internal_token(config.internal_token.clone().unwrap_or_default());
    Ok(Deployment {
        group,
        gateway,
        deployment_id: spec.deployment_id.to_owned(),
        asset_root,
        artifact_path: spec.artifact_path.to_owned(),
    })
}

fn free_loopback_port() -> Result<SocketAddr> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to reserve a loopback port")?;
    let address = listener
        .local_addr()
        .context("failed to read loopback port")?;
    drop(listener);
    Ok(address)
}

fn validate_identifier(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("{label} must be 1-128 safe characters");
    }
    Ok(())
}

#[async_trait::async_trait]
impl UniversalStorefront for UniversalSsr {
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
        };
        let cache_key = cache_key(&request, store.store_id.as_str());
        if let Some(key) = &cache_key
            && let Some(response) = self.cached_response(key).await
        {
            return response;
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
        let context = SsrRequestContext {
            store_id: store.store_id.to_string(),
            deployment_id: deployment.deployment_id.clone(),
            request_id: request_id.unwrap_or_else(|| "ssr-preview".to_owned()),
            locale: Some(store.locale),
            currency: Some(store.currency.to_string()),
            market_id: None,
            scheme,
            preview: true,
        };
        match deployment
            .gateway
            .forward_streaming(request, &context)
            .await
        {
            Ok(response) => into_response(response),
            Err(error) => error_response(&error),
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

async fn serve_asset(
    root: &std::path::Path,
    path: &str,
    headers: &axum::http::HeaderMap,
) -> Response {
    use axum::http::header::{
        ACCEPT_RANGES, CACHE_CONTROL, CONTENT_RANGE, ETAG, IF_NONE_MATCH, RANGE,
    };

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
    let Ok(metadata) = tokio::fs::metadata(&full).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let len = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs());
    let etag = format!("\"{len:x}-{modified:x}\"");
    let cache_control = "public, max-age=31536000, immutable";
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, &etag)
            .header(CACHE_CONTROL, cache_control)
            .header(ACCEPT_RANGES, "bytes")
            .body(Body::empty())
            .unwrap_or_else(|_| StatusCode::NOT_MODIFIED.into_response());
    }
    let Ok(bytes) = tokio::fs::read(&full).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let range = headers
        .get(RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, len));
    let (status, body, content_range) = match range {
        Some(Ok((start, end))) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[start..=end].to_vec(),
            Some(format!("bytes {start}-{end}/{len}")),
        ),
        Some(Err(())) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(CONTENT_RANGE, format!("bytes */{len}"))
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        }
        None => (StatusCode::OK, bytes, None),
    };
    let mut builder = Response::builder()
        .status(status)
        .header(axum::http::header::CONTENT_TYPE, content_type(&full))
        .header(CACHE_CONTROL, cache_control)
        .header(ETAG, &etag)
        .header(ACCEPT_RANGES, "bytes");
    if let Some(content_range) = content_range {
        builder = builder.header(CONTENT_RANGE, content_range);
    }
    builder.body(Body::from(body)).unwrap_or_else(|error| {
        tracing::error!(error = %error, "failed to build asset response");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

/// Parses one byte range. Returns `Ok((start, end))` or `Err` when unsatisfiable.
fn parse_range(value: &str, len: u64) -> Option<Result<(usize, usize), ()>> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return Some(Err(()));
    }
    let (start, end) = value.split_once('-')?;
    if len == 0 {
        return Some(Err(()));
    }
    let (start, end) = if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return Some(Err(()));
        }
        let suffix = suffix.min(len);
        (len - suffix, len - 1)
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            len - 1
        } else {
            end.parse::<u64>().ok()?.min(len - 1)
        };
        if start > end || start >= len {
            return Some(Err(()));
        }
        (start, end)
    };
    Some(Ok((
        usize::try_from(start).ok()?,
        usize::try_from(end).ok()?,
    )))
}

fn cache_key(request: &axum::http::Request<Body>, store_id: &str) -> Option<String> {
    if request.method() != axum::http::Method::GET {
        return None;
    }
    if request
        .headers()
        .contains_key(axum::http::header::AUTHORIZATION)
        || request.headers().contains_key(axum::http::header::COOKIE)
        || request
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("no-cache"))
    {
        return None;
    }
    let language = request
        .headers()
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    Some(format!(
        "{store_id}:{}:{language}",
        request
            .uri()
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str)
    ))
}

fn cache_ttl(headers: &axum::http::HeaderMap) -> Option<Duration> {
    if headers.contains_key(axum::http::header::SET_COOKIE) {
        return None;
    }
    let value = headers
        .get(axum::http::header::CACHE_CONTROL)?
        .to_str()
        .ok()?;
    let lower = value.to_ascii_lowercase();
    if lower.contains("no-store") || lower.contains("private") {
        return None;
    }
    let mut ttl = None;
    for part in lower.split(',') {
        let part = part.trim();
        let Some(seconds) = part
            .strip_prefix("s-maxage=")
            .or_else(|| part.strip_prefix("max-age="))
        else {
            continue;
        };
        let Ok(seconds) = seconds.trim().parse::<u64>() else {
            continue;
        };
        ttl = Some(Duration::from_secs(seconds));
        if part.starts_with("s-maxage=") {
            break;
        }
    }
    ttl.filter(|ttl| !ttl.is_zero())
}

async fn buffer_body(body: SsrBody) -> Option<bytes::Bytes> {
    match body {
        SsrBody::Full(bytes) => (bytes.len() <= MAX_CACHE_ENTRY_BYTES).then_some(bytes),
        SsrBody::Stream(mut stream) => {
            let mut output = bytes::BytesMut::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.ok()?;
                if output.len() + chunk.len() > MAX_CACHE_ENTRY_BYTES {
                    return None;
                }
                output.extend_from_slice(&chunk);
            }
            Some(output.freeze())
        }
    }
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
        if !headers.contains_key(axum::http::header::CACHE_CONTROL) {
            headers.insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
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
    use crate::config::SsrDeploymentConfig;

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
    if (url.pathname === "/cached") {
      globalThis.__fastcartCacheCount = (globalThis.__fastcartCacheCount || 0) + 1;
      return Response.json(
        { count: globalThis.__fastcartCacheCount },
        { headers: { "cache-control": "public, max-age=60" } },
      );
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
            drain_timeout_ms: 100,
            artifact_max_bytes: 10 * 1024 * 1024,
            artifact_max_files: 100,
            artifact_root: None,
            deployments: Vec::new(),
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
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store"
        );
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["store"], "store_01");
        assert_eq!(value["deployment"], "deploy_server");
        assert_eq!(value["requestId"], "req_server");
        assert_eq!(value["proto"], "https");
        assert_eq!(value["token"], "server-secret");

        let first = runtime
            .render(
                Request::builder()
                    .uri("/cached")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_01".to_owned()),
                    domain: "shop.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                Some("cache_1".to_owned()),
                "https".to_owned(),
            )
            .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = axum::body::to_bytes(first.into_body(), 1024)
            .await
            .expect("first body");
        let first_value: serde_json::Value =
            serde_json::from_slice(&first_body).expect("first json");
        let second = runtime
            .render(
                Request::builder()
                    .uri("/cached")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_01".to_owned()),
                    domain: "shop.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                Some("cache_2".to_owned()),
                "https".to_owned(),
            )
            .await;
        assert_eq!(
            second.headers().get("x-fastcart-cache").expect("cache hit"),
            "HIT"
        );
        let second_body = axum::body::to_bytes(second.into_body(), 1024)
            .await
            .expect("second body");
        let second_value: serde_json::Value =
            serde_json::from_slice(&second_body).expect("second json");
        assert_eq!(first_value["count"], second_value["count"]);

        runtime.invalidate_cache("store_01").await;
        let third = runtime
            .render(
                Request::builder()
                    .uri("/cached")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_01".to_owned()),
                    domain: "shop.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                Some("cache_3".to_owned()),
                "https".to_owned(),
            )
            .await;
        let third_body = axum::body::to_bytes(third.into_body(), 1024)
            .await
            .expect("third body");
        let third_value: serde_json::Value =
            serde_json::from_slice(&third_body).expect("third json");
        assert_ne!(first_value["count"], third_value["count"]);

        let asset = runtime
            .serve_asset(
                "store_01",
                "app.js".to_owned(),
                axum::http::HeaderMap::new(),
            )
            .await;
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
        let etag = asset
            .headers()
            .get(axum::http::header::ETAG)
            .expect("etag")
            .to_str()
            .expect("etag text")
            .to_owned();
        let asset_body = axum::body::to_bytes(asset.into_body(), 1024)
            .await
            .expect("asset body");
        assert_eq!(&asset_body[..], b"console.log('asset');");
        let mut conditional = axum::http::HeaderMap::new();
        conditional.insert(
            axum::http::header::IF_NONE_MATCH,
            axum::http::HeaderValue::from_str(&etag).expect("if-none-match"),
        );
        let not_modified = runtime
            .serve_asset("store_01", "app.js".to_owned(), conditional)
            .await;
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

        let mut range = axum::http::HeaderMap::new();
        range.insert(
            axum::http::header::RANGE,
            axum::http::HeaderValue::from_static("bytes=0-6"),
        );
        let partial = runtime
            .serve_asset("store_01", "app.js".to_owned(), range)
            .await;
        assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            partial
                .headers()
                .get(axum::http::header::CONTENT_RANGE)
                .unwrap(),
            "bytes 0-6/21"
        );
        let partial_body = axum::body::to_bytes(partial.into_body(), 1024)
            .await
            .expect("partial body");
        assert_eq!(&partial_body[..], b"console");

        let traversal = runtime
            .serve_asset(
                "store_01",
                "../fastcart-ssr.json".to_owned(),
                axum::http::HeaderMap::new(),
            )
            .await;
        assert_eq!(traversal.status(), StatusCode::NOT_FOUND);
        let missing = runtime
            .serve_asset(
                "store_01",
                "missing.js".to_owned(),
                axum::http::HeaderMap::new(),
            )
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        runtime.shutdown().await;
    }

    async fn render_store(runtime: &UniversalSsr, store_id: &str) -> serde_json::Value {
        let response = runtime
            .render(
                Request::builder()
                    .uri("/context")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string(store_id.to_owned()),
                    domain: format!("{store_id}.example"),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                Some(format!("req_{store_id}")),
                "https".to_owned(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&body).expect("json")
    }

    async fn render_preview_store(runtime: &UniversalSsr, store_id: &str) -> serde_json::Value {
        let response = runtime
            .render_preview(
                Request::builder()
                    .uri("/context")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string(store_id.to_owned()),
                    domain: format!("{store_id}.example"),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                Some(format!("preview_{store_id}")),
                "https".to_owned(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        serde_json::from_slice(&body).expect("json")
    }

    #[tokio::test]
    async fn routes_stores_to_isolated_deployments_and_assets() {
        let Some(binary) = workerd_binary() else {
            eprintln!("skipping: no workerd binary");
            return;
        };
        let root = tempfile::tempdir().expect("root");
        let source_a = artifact(&root.path().join("a"));
        let source_b = artifact(&root.path().join("b"));
        std::fs::write(source_a.join("client/app.js"), "asset-a").expect("asset a");
        std::fs::write(source_a.join("client/old.js"), "old-a").expect("old asset a");
        std::fs::write(source_b.join("client/app.js"), "asset-b").expect("asset b");
        let config = SsrConfig {
            enabled: true,
            binary,
            artifact_path: std::path::PathBuf::new(),
            runtime_dir: root.path().join("runtime"),
            listen: format!("127.0.0.1:{}", free_port())
                .parse()
                .expect("listen"),
            deployment_id: "unused".to_owned(),
            internal_token: Some("server-secret".to_owned()),
            request_timeout_ms: 10_000,
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 5_000,
            restart_backoff_ms: 10,
            max_restarts: 1,
            drain_timeout_ms: 100,
            artifact_max_bytes: 10 * 1024 * 1024,
            artifact_max_files: 100,
            artifact_root: Some(root.path().to_path_buf()),
            deployments: vec![
                SsrDeploymentConfig {
                    store_id: "store_a".to_owned(),
                    deployment_id: "deploy_a".to_owned(),
                    artifact_path: source_a.clone(),
                    listen: Some(
                        format!("127.0.0.1:{}", free_port())
                            .parse()
                            .expect("listen a"),
                    ),
                    preview: false,
                },
                SsrDeploymentConfig {
                    store_id: "store_b".to_owned(),
                    deployment_id: "deploy_b".to_owned(),
                    artifact_path: source_b.clone(),
                    listen: Some(
                        format!("127.0.0.1:{}", free_port())
                            .parse()
                            .expect("listen b"),
                    ),
                    preview: false,
                },
            ],
        };
        let runtime = UniversalSsr::initialize(&config)
            .await
            .expect("initialize")
            .expect("enabled");

        for (store_id, deployment_id) in [("store_a", "deploy_a"), ("store_b", "deploy_b")] {
            let request = Request::builder()
                .uri("/context")
                .body(Body::empty())
                .expect("request");
            let store = StoreContext {
                store_id: StoreId::from_string(store_id.to_owned()),
                domain: format!("{store_id}.example"),
                locale: "en-US".to_owned(),
                currency: CurrencyCode::new("USD").expect("currency"),
                timezone: "UTC".to_owned(),
            };
            let response = runtime
                .render(
                    request,
                    store,
                    Some(format!("req_{store_id}")),
                    "https".to_owned(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body");
            let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
            assert_eq!(value["store"], store_id);
            assert_eq!(value["deployment"], deployment_id);
        }

        let missing = runtime
            .render(
                Request::builder()
                    .uri("/context")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_missing".to_owned()),
                    domain: "missing.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                None,
                "https".to_owned(),
            )
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        for (store_id, expected) in [("store_a", "asset-a"), ("store_b", "asset-b")] {
            let asset = runtime
                .serve_asset(store_id, "app.js".to_owned(), axum::http::HeaderMap::new())
                .await;
            assert_eq!(asset.status(), StatusCode::OK);
            let body = axum::body::to_bytes(asset.into_body(), 1024)
                .await
                .expect("asset");
            assert_eq!(&body[..], expected.as_bytes());
        }

        let source_c = artifact(&root.path().join("c"));
        std::fs::write(source_c.join("client/app.js"), "asset-c").expect("asset c");
        runtime
            .activate("store_a", "deploy_a2", &source_c)
            .await
            .expect("activate");
        let activated = render_store(&runtime, "store_a").await;
        assert_eq!(activated["deployment"], "deploy_a2");
        let asset = runtime
            .serve_asset("store_a", "app.js".to_owned(), axum::http::HeaderMap::new())
            .await;
        let body = axum::body::to_bytes(asset.into_body(), 1024)
            .await
            .expect("asset");
        assert_eq!(&body[..], b"asset-c");

        assert!(
            runtime
                .activate("store_a", "deploy_bad", &root.path().join("missing"))
                .await
                .is_err()
        );
        let still_active = render_store(&runtime, "store_a").await;
        assert_eq!(still_active["deployment"], "deploy_a2");

        runtime.rollback("store_a").await.expect("rollback");
        let rolled_back = render_store(&runtime, "store_a").await;
        assert_eq!(rolled_back["deployment"], "deploy_a");

        runtime
            .activate_preview("store_a", "deploy_preview", &source_b)
            .await
            .expect("activate preview");
        assert!(runtime.has_preview("store_a").await);
        let preview = render_preview_store(&runtime, "store_a").await;
        assert_eq!(preview["deployment"], "deploy_preview");
        let production = render_store(&runtime, "store_a").await;
        assert_eq!(production["deployment"], "deploy_a");
        runtime
            .deactivate_preview("store_a")
            .await
            .expect("deactivate preview");
        assert!(!runtime.has_preview("store_a").await);
        let missing_preview = runtime
            .render_preview(
                Request::builder()
                    .uri("/context")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_a".to_owned()),
                    domain: "store_a.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                None,
                "https".to_owned(),
            )
            .await;
        assert_eq!(missing_preview.status(), StatusCode::NOT_FOUND);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn activation_state_survives_restart() {
        let Some(binary) = workerd_binary() else {
            eprintln!("skipping: no workerd binary");
            return;
        };
        let root = tempfile::tempdir().expect("root");
        let source_a = artifact(&root.path().join("a"));
        let source_b = artifact(&root.path().join("b"));
        let runtime_dir = root.path().join("runtime");
        let base = SsrConfig {
            enabled: true,
            binary,
            artifact_path: source_a,
            runtime_dir: runtime_dir.clone(),
            listen: format!("127.0.0.1:{}", free_port())
                .parse()
                .expect("listen"),
            deployment_id: "default".to_owned(),
            internal_token: Some("server-secret".to_owned()),
            request_timeout_ms: 10_000,
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 5_000,
            restart_backoff_ms: 10,
            max_restarts: 1,
            drain_timeout_ms: 100,
            artifact_max_bytes: 10 * 1024 * 1024,
            artifact_max_files: 100,
            artifact_root: Some(root.path().to_path_buf()),
            deployments: Vec::new(),
        };
        let runtime = UniversalSsr::initialize(&base)
            .await
            .expect("initialize")
            .expect("enabled");
        runtime
            .activate("store_persist", "deploy_persisted", &source_b)
            .await
            .expect("activate");
        runtime.shutdown().await;

        let restarted = UniversalSsr::initialize(&base)
            .await
            .expect("restart")
            .expect("enabled");
        let value = render_store(&restarted, "store_persist").await;
        assert_eq!(value["deployment"], "deploy_persisted");
        let missing = restarted
            .render(
                Request::builder()
                    .uri("/context")
                    .body(Body::empty())
                    .expect("request"),
                StoreContext {
                    store_id: StoreId::from_string("store_other".to_owned()),
                    domain: "other.example".to_owned(),
                    locale: "en-US".to_owned(),
                    currency: CurrencyCode::new("USD").expect("currency"),
                    timezone: "UTC".to_owned(),
                },
                None,
                "https".to_owned(),
            )
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        restarted.shutdown().await;
    }
}
