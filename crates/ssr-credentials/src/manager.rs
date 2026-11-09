//! In-memory credential manager with optional backend persistence.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use fastcart_http::Metrics;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::time::MissedTickBehavior;

use crate::backend::{BackendError, CredentialBackend};
use crate::model::{CredentialSnapshot, CredentialToken};

/// Whether the manager may write to its backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialMode {
    /// External source of truth; rotate/revoke are rejected.
    ReadOnly,
    /// The manager may persist rotations and revocations.
    ReadWrite,
}

/// Errors returned by credential operations.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// Backend failure.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// The manager is configured read-only.
    #[error("credential manager is read-only")]
    ReadOnly,
    /// The credential lock was poisoned.
    #[error("credential lock poisoned")]
    LockPoisoned,
}

/// Shared credential snapshot and optional persistence backend.
#[derive(Clone)]
pub struct CredentialManager {
    snapshot: Arc<RwLock<Arc<CredentialSnapshot>>>,
    backend: Option<Arc<dyn CredentialBackend>>,
    mode: CredentialMode,
    metrics: Option<Arc<Metrics>>,
}

impl std::fmt::Debug for CredentialManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CredentialManager")
            .field(
                "tokens",
                &self
                    .snapshot
                    .read()
                    .map_or(0, |snapshot| snapshot.tokens.len()),
            )
            .field("backend", &self.backend.is_some())
            .field("mode", &self.mode)
            .finish()
    }
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl CredentialManager {
    /// Creates an in-memory, writable manager. Rotations are not durable.
    #[must_use]
    pub fn new(tokens: Vec<CredentialToken>) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(Arc::new(CredentialSnapshot {
                revision: None,
                tokens,
            }))),
            backend: None,
            mode: CredentialMode::ReadWrite,
            metrics: None,
        }
    }

    /// Loads a snapshot from `backend`, seeding it from `configured` only when
    /// the backend has no state and `seed_from_config` is set.
    pub async fn load(
        backend: Arc<dyn CredentialBackend>,
        configured: Vec<CredentialToken>,
        seed_from_config: bool,
        mode: CredentialMode,
    ) -> Result<Self, CredentialError> {
        Self::load_with_metrics(backend, configured, seed_from_config, mode, None).await
    }

    /// Loads a snapshot and records backend metrics.
    pub async fn load_with_metrics(
        backend: Arc<dyn CredentialBackend>,
        configured: Vec<CredentialToken>,
        seed_from_config: bool,
        mode: CredentialMode,
        metrics: Option<Arc<Metrics>>,
    ) -> Result<Self, CredentialError> {
        let loaded = match backend.load().await {
            Ok(loaded) => {
                if let Some(metrics) = &metrics {
                    metrics.record_ssr_credential_load(true);
                }
                loaded
            }
            Err(error) => {
                if let Some(metrics) = &metrics {
                    metrics.record_ssr_credential_load(false);
                }
                return Err(error.into());
            }
        };
        let mut snapshot = match loaded {
            Some(snapshot) => snapshot,
            None => CredentialSnapshot {
                revision: None,
                tokens: if seed_from_config {
                    configured
                } else {
                    Vec::new()
                },
            },
        };
        if snapshot.revision.is_none()
            && seed_from_config
            && !snapshot.tokens.is_empty()
            && mode == CredentialMode::ReadWrite
        {
            let revision = match backend.save(&snapshot, None).await {
                Ok(revision) => {
                    if let Some(metrics) = &metrics {
                        metrics.record_ssr_credential_save(true);
                    }
                    revision
                }
                Err(error) => {
                    if let Some(metrics) = &metrics {
                        metrics.record_ssr_credential_save(false);
                    }
                    return Err(error.into());
                }
            };
            snapshot.revision = Some(revision);
        }
        let manager = Self {
            snapshot: Arc::new(RwLock::new(Arc::new(snapshot))),
            backend: Some(backend),
            mode,
            metrics,
        };
        manager.publish_metrics();
        Ok(manager)
    }

    /// Attaches a metrics collector.
    pub fn set_metrics(&mut self, metrics: Arc<Metrics>) {
        self.metrics = Some(metrics);
        self.publish_metrics();
    }

    fn publish_metrics(&self) {
        if let Some(metrics) = &self.metrics
            && let Ok(snapshot) = self.snapshot.read()
        {
            metrics.set_ssr_credential_tokens(snapshot.tokens.len());
        }
    }

    /// Loads a file-backed manager.
    pub async fn load_file(
        path: impl Into<std::path::PathBuf>,
        configured: Vec<CredentialToken>,
        read_only: bool,
        seed_from_config: bool,
    ) -> Result<Self, CredentialError> {
        let mode = if read_only {
            CredentialMode::ReadOnly
        } else {
            CredentialMode::ReadWrite
        };
        let backend = Arc::new(crate::backends::file::FileCredentialBackend::new(
            path, read_only,
        ));
        Self::load(backend, configured, seed_from_config, mode).await
    }

    /// Whether no credentials are currently configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snapshot
            .read()
            .map_or(true, |snapshot| snapshot.tokens.is_empty())
    }

    /// Validates a bearer token in constant time.
    #[must_use]
    pub fn validate(&self, provided: &str) -> Option<CredentialToken> {
        let now = fastcart_kernel::now_timestamp();
        let snapshot = self.snapshot.read().ok()?;
        snapshot
            .tokens
            .iter()
            .filter(|token| token.expires_at.is_none_or(|expiry| expiry > now))
            .find(|token| bool::from(provided.as_bytes().ct_eq(token.token.as_bytes())))
            .cloned()
    }

    /// Returns the newest active token for a store and preview/production mode.
    #[must_use]
    pub fn token_for_store(&self, store_id: &str, preview: bool) -> Option<String> {
        let now = fastcart_kernel::now_timestamp();
        let snapshot = self.snapshot.read().ok()?;
        let active = |token: &&CredentialToken| {
            token.preview == preview && token.expires_at.is_none_or(|expiry| expiry > now)
        };
        snapshot
            .tokens
            .iter()
            .rev()
            .find(|token| token.store_id.as_deref() == Some(store_id) && active(token))
            .or_else(|| {
                if preview {
                    return None;
                }
                snapshot
                    .tokens
                    .iter()
                    .rev()
                    .find(|token| token.store_id.is_none() && active(token))
            })
            .map(|token| token.token.clone())
    }

    /// Rotates a store token, persisting before swapping the live snapshot.
    pub async fn rotate(
        &self,
        store_id: &str,
        token: String,
        expires_at: Option<i64>,
        grace_seconds: i64,
        preview: bool,
    ) -> Result<(), CredentialError> {
        if self.mode == CredentialMode::ReadOnly {
            return Err(CredentialError::ReadOnly);
        }
        let now = fastcart_kernel::now_timestamp();
        let deadline = now.saturating_add(grace_seconds.max(0));
        let current = self
            .snapshot
            .read()
            .map_err(|_| CredentialError::LockPoisoned)?
            .clone();
        let mut tokens = current.tokens.clone();
        for existing in &mut tokens {
            if existing.store_id.as_deref() == Some(store_id) && existing.preview == preview {
                existing.expires_at = Some(
                    existing
                        .expires_at
                        .map_or(deadline, |value| value.min(deadline)),
                );
            }
        }
        tokens.push(CredentialToken {
            token,
            store_id: Some(store_id.to_owned()),
            expires_at,
            preview,
        });
        let mut next = CredentialSnapshot {
            revision: current.revision.clone(),
            tokens,
        };
        if let Some(backend) = &self.backend {
            let revision = match backend.save(&next, current.revision.as_deref()).await {
                Ok(revision) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_ssr_credential_save(true);
                    }
                    revision
                }
                Err(error) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_ssr_credential_save(false);
                    }
                    return Err(error.into());
                }
            };
            next.revision = Some(revision);
        }
        *self
            .snapshot
            .write()
            .map_err(|_| CredentialError::LockPoisoned)? = Arc::new(next);
        self.publish_metrics();
        Ok(())
    }

    /// Revokes a scoped token, persisting before swapping the live snapshot.
    pub async fn revoke(&self, store_id: &str, token: &str) -> Result<bool, CredentialError> {
        if self.mode == CredentialMode::ReadOnly {
            return Err(CredentialError::ReadOnly);
        }
        let current = self
            .snapshot
            .read()
            .map_err(|_| CredentialError::LockPoisoned)?
            .clone();
        let before = current.tokens.len();
        let tokens: Vec<CredentialToken> = current
            .tokens
            .iter()
            .filter(|existing| {
                !(existing.store_id.as_deref() == Some(store_id)
                    && bool::from(token.as_bytes().ct_eq(existing.token.as_bytes())))
            })
            .cloned()
            .collect();
        let removed = tokens.len() != before;
        let mut next = CredentialSnapshot {
            revision: current.revision.clone(),
            tokens,
        };
        if removed && let Some(backend) = &self.backend {
            let revision = match backend.save(&next, current.revision.as_deref()).await {
                Ok(revision) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_ssr_credential_save(true);
                    }
                    revision
                }
                Err(error) => {
                    if let Some(metrics) = &self.metrics {
                        metrics.record_ssr_credential_save(false);
                    }
                    return Err(error.into());
                }
            };
            next.revision = Some(revision);
        }
        *self
            .snapshot
            .write()
            .map_err(|_| CredentialError::LockPoisoned)? = Arc::new(next);
        self.publish_metrics();
        Ok(removed)
    }

    /// Reloads the backend and replaces the snapshot when its revision changed.
    pub async fn refresh(&self) -> Result<bool, CredentialError> {
        let Some(backend) = &self.backend else {
            return Ok(false);
        };
        let loaded = match backend.load().await {
            Ok(loaded) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_ssr_credential_load(true);
                }
                loaded
            }
            Err(error) => {
                if let Some(metrics) = &self.metrics {
                    metrics.record_ssr_credential_load(false);
                    metrics.record_ssr_credential_refresh(false, false);
                }
                return Err(error.into());
            }
        };
        let current = self
            .snapshot
            .read()
            .map_err(|_| CredentialError::LockPoisoned)?
            .clone();
        let Some(loaded) = loaded else {
            if current.tokens.is_empty() {
                if let Some(metrics) = &self.metrics {
                    metrics.record_ssr_credential_refresh(true, false);
                }
                return Ok(false);
            }
            *self
                .snapshot
                .write()
                .map_err(|_| CredentialError::LockPoisoned)? = Arc::new(CredentialSnapshot {
                revision: None,
                tokens: Vec::new(),
            });
            self.publish_metrics();
            if let Some(metrics) = &self.metrics {
                metrics.record_ssr_credential_refresh(true, true);
            }
            return Ok(true);
        };
        if loaded.revision == current.revision {
            if let Some(metrics) = &self.metrics {
                metrics.record_ssr_credential_refresh(true, false);
            }
            return Ok(false);
        }
        *self
            .snapshot
            .write()
            .map_err(|_| CredentialError::LockPoisoned)? = Arc::new(loaded);
        self.publish_metrics();
        if let Some(metrics) = &self.metrics {
            metrics.record_ssr_credential_refresh(true, true);
        }
        Ok(true)
    }

    /// Starts a background refresh loop. The task ends with the process.
    pub fn spawn_refresh(&self, interval: Duration) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                if let Err(error) = manager.refresh().await {
                    tracing::warn!(error = %error, "SSR credential refresh failed");
                }
            }
        });
    }
}
