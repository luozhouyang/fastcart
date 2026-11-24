//! The SSR service: deployment registry, lifecycle and request path.
//!
//! [`SsrService`] decides which artifact serves a store, performs activation
//! and rollback, keeps previews apart from live deployments, and serves
//! rendered responses and static assets. It drives `fastcart-ssr-workerd` for
//! the process and gateway details.

use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use axum::body::Body;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use fastcart_http::Metrics;
use fastcart_kernel::StoreContext;
use fastcart_ssr_credentials::CredentialManager;
use fastcart_ssr_workerd::artifact::{ArtifactLimits, ArtifactStore};
use fastcart_ssr_workerd::{
    RuntimeGroupId, RuntimeManager, RuntimeManagerConfig, SsrBody, SsrError, SsrGateway,
    SsrRequestContext, SsrResponse,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};

use super::config::SsrConfig;
use crate::contract::UniversalStorefront;

mod serving;
mod universal;

/// One active storefront deployment.
#[derive(Clone)]
struct Deployment {
    group: RuntimeGroupId,
    gateway: SsrGateway,
    deployment_id: String,
    asset_root: PathBuf,
    artifact_dir: PathBuf,
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

/// Storefront SSR service shared by every store.
pub struct SsrService {
    manager: Arc<Mutex<RuntimeManager>>,
    deployments: RwLock<HashMap<String, Arc<Deployment>>>,
    previews: RwLock<HashMap<String, Arc<Deployment>>>,
    history: RwLock<HashMap<String, Vec<DeploymentMeta>>>,
    asset_roots: Arc<RwLock<Vec<PathBuf>>>,
    preview_asset_roots: Arc<RwLock<Vec<PathBuf>>>,
    artifact_refs: Arc<RwLock<HashMap<PathBuf, usize>>>,
    response_cache: RwLock<HashMap<String, CachedResponse>>,
    default: Option<Arc<Deployment>>,
    metrics: Arc<Metrics>,
    credentials: CredentialManager,
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
    internal_token: Option<String>,
}

impl SsrService {
    /// Imports configured artifacts and starts one workerd group per deployment.
    pub async fn initialize(
        config: &SsrConfig,
        metrics: Arc<Metrics>,
        credentials: CredentialManager,
    ) -> Result<Option<Arc<Self>>> {
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
                        internal_token: None,
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
        let preview_asset_roots: Vec<PathBuf> = previews
            .values()
            .map(|deployment| deployment.asset_root.clone())
            .collect();
        let mut artifact_refs: HashMap<PathBuf, usize> = HashMap::new();
        for deployment in deployments
            .values()
            .chain(previews.values())
            .chain(default.iter())
        {
            *artifact_refs
                .entry(deployment.artifact_dir.clone())
                .or_default() += 1;
        }
        let active_artifacts: HashSet<PathBuf> = artifact_refs.keys().cloned().collect();
        sweep_orphan_artifacts(artifacts.root(), &active_artifacts).await;
        metrics.set_ssr_deployments(deployments.len() + previews.len());
        Ok(Some(Arc::new(Self {
            manager: Arc::new(Mutex::new(manager)),
            deployments: RwLock::new(deployments),
            previews: RwLock::new(previews),
            history: RwLock::new(HashMap::new()),
            asset_roots: Arc::new(RwLock::new(asset_roots)),
            preview_asset_roots: Arc::new(RwLock::new(preview_asset_roots)),
            artifact_refs: Arc::new(RwLock::new(artifact_refs)),
            response_cache: RwLock::new(HashMap::new()),
            default,
            metrics,
            credentials,
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
            internal_token: self.config.internal_token.clone(),
        };
        let limits = ArtifactLimits {
            max_bytes: self.config.artifact_max_bytes,
            max_files: self.config.artifact_max_files,
        };
        // The manager lock is held across the await on purpose, and it is the
        // one guard in this service that is not merely protecting a map.
        // `start_deployment` needs `&mut RuntimeManager` because the manager is
        // the state it mutates — there is no inner mutability to borrow
        // instead — and spinning up a workerd group is not something two
        // deployments should interleave. Serving requests does not take this
        // lock: it reads `deployments`/`previews` under their own RwLock, so the
        // only thing serialized here is deployment against deployment.
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
        self.retain_artifact(&deployment).await;
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
        self.refresh_deployment_metric().await;
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
                self.schedule_drain(previous.clone());
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

    async fn retain_artifact(&self, deployment: &Deployment) {
        *self
            .artifact_refs
            .write()
            .await
            .entry(deployment.artifact_dir.clone())
            .or_default() += 1;
    }

    fn schedule_drain(&self, deployment: Arc<Deployment>) {
        let manager = Arc::clone(&self.manager);
        let refs = Arc::clone(&self.artifact_refs);
        let roots = Arc::clone(&self.asset_roots);
        let preview_roots = Arc::clone(&self.preview_asset_roots);
        let artifact_dir = deployment.artifact_dir.clone();
        let group = deployment.group.clone();
        let delay = Duration::from_millis(self.config.drain_timeout_ms.max(1));
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // Same reasoning as in `replace`: tearing a group down mutates the
            // manager, and it happens after the drain delay so a request that
            // is still in flight is not cut off mid-response.
            match manager.lock().await.stop_group(&group).await {
                Ok(()) => {
                    release_artifact(artifact_dir, refs, roots, preview_roots).await;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        group = %group,
                        "failed to drain SSR group; retaining its artifact"
                    );
                }
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
            internal_token: None,
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
        self.retain_artifact(&deployment).await;
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
            let mut preview_roots = self.preview_asset_roots.write().await;
            if !preview_roots.iter().any(|root| root == &old.asset_root) {
                preview_roots.push(old.asset_root.clone());
            }
            drop(preview_roots);
            self.schedule_drain(old.clone());
        }
        self.refresh_deployment_metric().await;
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
        let mut preview_roots = self.preview_asset_roots.write().await;
        if !preview_roots.iter().any(|root| root == &old.asset_root) {
            preview_roots.push(old.asset_root.clone());
        }
        drop(preview_roots);
        self.schedule_drain(old.clone());
        self.refresh_deployment_metric().await;
        self.persist_state().await;
        Ok(())
    }

    async fn refresh_deployment_metric(&self) {
        let count = self.deployments.read().await.len() + self.previews.read().await.len();
        self.metrics.set_ssr_deployments(count);
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
        self.metrics.set_ssr_deployments(0);
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
            internal_token: config.internal_token.clone(),
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
                internal_token: if deployment.preview {
                    deployment.internal_token.clone()
                } else {
                    deployment
                        .internal_token
                        .clone()
                        .or_else(|| config.internal_token.clone())
                },
            }
        })
        .collect()
}

async fn start_deployment(
    manager: &mut RuntimeManager,
    store: &ArtifactStore,
    limits: ArtifactLimits,
    _config: &SsrConfig,
    spec: &DeploymentSpec<'_>,
    group: RuntimeGroupId,
) -> Result<Deployment> {
    let artifact = store.import(spec.artifact_path, limits).with_context(|| {
        format!(
            "failed to import the SSR artifact for {}",
            spec.store_id.unwrap_or("default")
        )
    })?;
    let artifact_dir = artifact.root().to_path_buf();
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
    if health.status != fastcart_ssr_workerd::RuntimeStatus::Healthy {
        let _ = manager.stop_group(&group).await;
        bail!("SSR runtime started but did not become healthy");
    }
    let gateway = manager
        .gateway(&group)
        .context("failed to create the SSR gateway")?
        .with_internal_token(spec.internal_token.clone().unwrap_or_default());
    Ok(Deployment {
        group,
        gateway,
        deployment_id: spec.deployment_id.to_owned(),
        asset_root,
        artifact_dir,
        artifact_path: spec.artifact_path.to_owned(),
    })
}

async fn sweep_orphan_artifacts(root: &Path, active: &HashSet<PathBuf>) {
    let entries = match tokio::fs::read_dir(root).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(error = %error, path = %root.display(), "failed to scan SSR artifacts");
            return;
        }
    };
    let mut entries = entries;
    let mut removed = 0_usize;
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(error = %error, path = %root.display(), "failed to read SSR artifact entry");
                break;
            }
        };
        let path = entry.path();
        if active.contains(&path) {
            continue;
        }
        let is_dir = entry
            .file_type()
            .await
            .is_ok_and(|file_type| file_type.is_dir());
        if !is_dir {
            continue;
        }
        match tokio::fs::remove_dir_all(&path).await {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(error = %error, path = %path.display(), "failed to remove orphaned SSR artifact");
            }
        }
    }
    if removed > 0 {
        tracing::info!(removed, path = %root.display(), "swept orphaned SSR artifacts");
    }
}

async fn release_artifact(
    artifact_dir: PathBuf,
    refs: Arc<RwLock<HashMap<PathBuf, usize>>>,
    roots: Arc<RwLock<Vec<PathBuf>>>,
    preview_roots: Arc<RwLock<Vec<PathBuf>>>,
) {
    let remaining = {
        let mut refs = refs.write().await;
        let Some(count) = refs.get_mut(&artifact_dir) else {
            return;
        };
        *count = count.saturating_sub(1);
        let remaining = *count;
        if remaining == 0 {
            refs.remove(&artifact_dir);
        }
        remaining
    };
    if remaining > 0 {
        return;
    }
    roots
        .write()
        .await
        .retain(|root| !root.starts_with(&artifact_dir));
    preview_roots
        .write()
        .await
        .retain(|root| !root.starts_with(&artifact_dir));
    match tokio::fs::remove_dir_all(&artifact_dir).await {
        Ok(()) => tracing::info!(path = %artifact_dir.display(), "removed retired SSR artifact"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(error = %error, path = %artifact_dir.display(), "failed to remove retired SSR artifact");
        }
    }
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

#[cfg(test)]
mod tests;
