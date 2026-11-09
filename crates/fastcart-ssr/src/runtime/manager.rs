//! Restart-aware supervisor for one or more workerd runtime groups.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::SsrError;
use crate::artifact::Artifact;
use crate::process::{RuntimeHealth, RuntimeStatus, WorkerdProcess};
use crate::runtime::config_generator::generate_workerd_config;

/// Stable identity of one runtime group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeGroupId(String);

impl RuntimeGroupId {
    /// Creates a group id from a safe name.
    pub fn new(value: impl Into<String>) -> Result<Self, SsrError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(SsrError::InvalidRuntimeConfig(
                "runtime group id must be 1-64 safe characters".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Group name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RuntimeGroupId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Manager-wide settings.
#[derive(Clone, Debug)]
pub struct RuntimeManagerConfig {
    /// Directory containing generated groups and runtime state.
    pub root: PathBuf,
    /// Pinned workerd binary.
    pub binary: PathBuf,
    /// Startup readiness deadline.
    pub startup_timeout: Duration,
    /// Graceful shutdown deadline.
    pub shutdown_timeout: Duration,
    /// Gateway request timeout.
    pub request_timeout: Duration,
    /// Minimum delay between starts of the same group.
    pub restart_backoff: Duration,
    /// Maximum automatic restarts before a group is marked failed.
    pub max_restarts: u32,
    /// Number of retained log lines.
    pub log_lines: usize,
}

impl RuntimeManagerConfig {
    /// Creates defaults rooted at `root` with the pinned binary.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, binary: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            binary: binary.into(),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            restart_backoff: Duration::from_secs(1),
            max_restarts: 3,
            log_lines: 200,
        }
    }
}

/// One managed runtime group.
pub struct RuntimeManager {
    config: RuntimeManagerConfig,
    groups: HashMap<RuntimeGroupId, ManagedGroup>,
}

struct ManagedGroup {
    artifact: Artifact,
    listen_addr: SocketAddr,
    process: Option<WorkerdProcess>,
    status: RuntimeStatus,
    restart_count: u32,
    last_started: Option<Instant>,
}

impl RuntimeManager {
    /// Creates a manager and its runtime directory.
    pub fn new(config: RuntimeManagerConfig) -> Result<Self, SsrError> {
        std::fs::create_dir_all(&config.root)
            .map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
        Ok(Self {
            config,
            groups: HashMap::new(),
        })
    }

    /// Starts a new group.
    pub async fn start_group(
        &mut self,
        id: RuntimeGroupId,
        artifact: Artifact,
        listen_addr: SocketAddr,
    ) -> Result<RuntimeHealth, SsrError> {
        if self.groups.contains_key(&id) {
            return Err(SsrError::AlreadyRunning(id.to_string()));
        }
        let mut process = self.start_process(&id, &artifact, listen_addr).await?;
        let health = process.health().await?;
        self.groups.insert(
            id,
            ManagedGroup {
                artifact,
                listen_addr,
                process: Some(process),
                status: RuntimeStatus::Healthy,
                restart_count: 0,
                last_started: Some(Instant::now()),
            },
        );
        Ok(health)
    }

    /// Stops a group without removing its artifact or restart counter.
    pub async fn stop_group(&mut self, id: &RuntimeGroupId) -> Result<(), SsrError> {
        let group = self
            .groups
            .get_mut(id)
            .ok_or_else(|| SsrError::UnknownGroup(id.to_string()))?;
        if let Some(mut process) = group.process.take() {
            process.stop().await?;
        }
        group.status = RuntimeStatus::Stopped;
        Ok(())
    }

    /// Restarts a group, applying the configured backoff and restart limit.
    pub async fn restart_group(&mut self, id: &RuntimeGroupId) -> Result<RuntimeHealth, SsrError> {
        let (artifact, listen_addr, restart_count, last_started) = {
            let group = self
                .groups
                .get(id)
                .ok_or_else(|| SsrError::UnknownGroup(id.to_string()))?;
            (
                group.artifact.clone(),
                group.listen_addr,
                group.restart_count,
                group.last_started,
            )
        };
        if restart_count >= self.config.max_restarts {
            if let Some(group) = self.groups.get_mut(id) {
                group.status = RuntimeStatus::Failed;
            }
            return Err(SsrError::RestartLimit(id.to_string()));
        }
        if let Some(started) = last_started {
            let elapsed = started.elapsed();
            if elapsed < self.config.restart_backoff {
                tokio::time::sleep(self.config.restart_backoff - elapsed).await;
            }
        }
        self.stop_group(id).await?;
        if let Some(group) = self.groups.get_mut(id) {
            group.status = RuntimeStatus::Restarting;
        }
        let mut process = match self.start_process(id, &artifact, listen_addr).await {
            Ok(process) => process,
            Err(error) => {
                if let Some(group) = self.groups.get_mut(id) {
                    group.status = RuntimeStatus::Failed;
                }
                return Err(error);
            }
        };
        let health = process.health().await?;
        if let Some(group) = self.groups.get_mut(id) {
            group.process = Some(process);
            group.status = RuntimeStatus::Healthy;
            group.restart_count = group.restart_count.saturating_add(1);
            group.last_started = Some(Instant::now());
        }
        Ok(health)
    }

    /// Probes one group and updates its tracked state.
    pub async fn health(&mut self, id: &RuntimeGroupId) -> Result<RuntimeHealth, SsrError> {
        let group = self
            .groups
            .get_mut(id)
            .ok_or_else(|| SsrError::UnknownGroup(id.to_string()))?;
        let Some(process) = group.process.as_mut() else {
            return Ok(RuntimeHealth {
                status: group.status,
                process_id: None,
                uptime_seconds: 0,
                last_error: None,
            });
        };
        let health = process.health().await?;
        group.status = match health.status {
            RuntimeStatus::Healthy => RuntimeStatus::Healthy,
            RuntimeStatus::Stopped => RuntimeStatus::Failed,
            status => status,
        };
        Ok(health)
    }

    /// Creates a gateway for a running group.
    pub fn gateway(&self, id: &RuntimeGroupId) -> Result<crate::gateway::SsrGateway, SsrError> {
        let group = self
            .groups
            .get(id)
            .ok_or_else(|| SsrError::UnknownGroup(id.to_string()))?;
        group
            .process
            .as_ref()
            .ok_or_else(|| SsrError::UnknownGroup(id.to_string()))?
            .gateway()
    }

    /// Current tracked status without probing.
    #[must_use]
    pub fn status(&self, id: &RuntimeGroupId) -> Option<RuntimeStatus> {
        self.groups.get(id).map(|group| group.status)
    }

    async fn start_process(
        &self,
        id: &RuntimeGroupId,
        artifact: &Artifact,
        listen_addr: SocketAddr,
    ) -> Result<WorkerdProcess, SsrError> {
        let config_dir = self.config.root.join("groups").join(id.as_str());
        let mut config =
            generate_workerd_config(&config_dir, artifact, listen_addr, &self.config.binary)?;
        config.startup_timeout = self.config.startup_timeout;
        config.shutdown_timeout = self.config.shutdown_timeout;
        config.request_timeout = self.config.request_timeout;
        config.log_lines = self.config.log_lines;
        WorkerdProcess::start(config).await
    }
}
