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
use crate::contract::{CapabilityGrants, UniversalStorefront};

mod cache;
mod lifecycle;
mod preview;
mod serving;
mod universal;

/// One active storefront deployment.
#[derive(Clone)]
pub(crate) struct Deployment {
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
    async fn cached_response(&self, key: &str) -> Option<Response> {
        let entry = {
            let cache = self.response_cache.read().await;
            let entry = cache.get(key)?.clone();
            if entry.stored.elapsed() > entry.ttl {
                drop(cache);
                // An expired entry is only removed on the slow path, so the
                // common hit never takes the write lock.
                self.response_cache.write().await.remove(key);
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
                axum::http::HeaderValue::from(entry.stored.elapsed().as_secs()),
            );
        }
        builder.body(Body::from(entry.body)).ok()
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
