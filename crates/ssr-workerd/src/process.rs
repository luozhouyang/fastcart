//! Workerd process lifecycle and health.

use std::collections::VecDeque;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::WorkerdConfig;
use crate::error::SsrError;
use crate::gateway::SsrGateway;

/// Lifecycle state reported by a workerd process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// The process was started but has not passed its first health probe.
    Starting,
    /// The process is running and its health endpoint is ready.
    Healthy,
    /// The process is running but its health probe failed.
    Unhealthy,
    /// The supervisor is restarting the process.
    Restarting,
    /// The process is draining in-flight requests.
    Draining,
    /// The process stopped and will not be restarted automatically.
    Failed,
    /// The process is not running.
    Stopped,
}

/// Snapshot of a workerd process's health.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHealth {
    /// Current lifecycle state.
    pub status: RuntimeStatus,
    /// OS process id when running.
    pub process_id: Option<u32>,
    /// Seconds since the process was spawned.
    pub uptime_seconds: u64,
    /// Last health error, when unhealthy.
    pub last_error: Option<String>,
}

/// One supervised workerd process.
pub struct WorkerdProcess {
    config: WorkerdConfig,
    child: Option<Child>,
    logs: Arc<Mutex<VecDeque<String>>>,
    client: reqwest::Client,
    started_at: Option<Instant>,
}

impl WorkerdProcess {
    /// Spawns workerd and waits until its readiness probe succeeds.
    pub async fn start(config: WorkerdConfig) -> Result<Self, SsrError> {
        config.validate()?;
        let mut child = Command::new(&config.binary)
            .arg("serve")
            .arg(&config.config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| SsrError::Start(error.to_string()))?;

        let logs = Arc::new(Mutex::new(VecDeque::with_capacity(config.log_lines)));
        if let Some(stdout) = child.stdout.take() {
            capture_lines(stdout, Arc::clone(&logs), config.log_lines, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            capture_lines(stderr, Arc::clone(&logs), config.log_lines, "stderr");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| SsrError::Start(error.to_string()))?;

        let mut process = Self {
            config,
            child: Some(child),
            logs,
            client,
            started_at: Some(Instant::now()),
        };

        if let Err(error) = process.wait_ready().await {
            process.stop().await.ok();
            return Err(error);
        }
        Ok(process)
    }

    /// Current configuration.
    #[must_use]
    pub fn config(&self) -> &WorkerdConfig {
        &self.config
    }

    /// Creates a gateway that targets this process.
    pub fn gateway(&self) -> Result<SsrGateway, SsrError> {
        SsrGateway::new(self.config.base_url(), self.config.request_timeout)
    }

    /// Returns whether the child process is still running.
    pub fn is_running(&mut self) -> bool {
        let state = self.child.as_mut().map(Child::try_wait);
        match state {
            Some(Ok(None)) => true,
            Some(Ok(Some(_)) | Err(_)) => {
                self.child = None;
                self.started_at = None;
                false
            }
            None => false,
        }
    }

    /// Probes the health endpoint and reports the process state.
    pub async fn health(&mut self) -> Result<RuntimeHealth, SsrError> {
        if !self.is_running() {
            return Ok(RuntimeHealth {
                status: RuntimeStatus::Stopped,
                process_id: None,
                uptime_seconds: 0,
                last_error: None,
            });
        }
        let process_id = self.child.as_ref().and_then(Child::id);
        let uptime_seconds = self
            .started_at
            .map_or(0, |started| started.elapsed().as_secs());
        match self.probe().await {
            Ok(()) => Ok(RuntimeHealth {
                status: RuntimeStatus::Healthy,
                process_id,
                uptime_seconds,
                last_error: None,
            }),
            Err(error) => Ok(RuntimeHealth {
                status: RuntimeStatus::Unhealthy,
                process_id,
                uptime_seconds,
                last_error: Some(error.to_string()),
            }),
        }
    }

    /// Sends SIGTERM, waits for the configured deadline, then force-kills.
    pub async fn stop(&mut self) -> Result<(), SsrError> {
        let Some(mut child) = self.child.take() else {
            self.started_at = None;
            return Ok(());
        };
        signal_terminate(&child);
        match tokio::time::timeout(self.config.shutdown_timeout, child.wait()).await {
            Ok(Ok(_)) => {
                self.started_at = None;
                Ok(())
            }
            Ok(Err(error)) => {
                self.started_at = None;
                Err(SsrError::Start(format!("failed to reap workerd: {error}")))
            }
            Err(_) => {
                child.start_kill().ok();
                child.wait().await.ok();
                self.started_at = None;
                Err(SsrError::ShutdownTimeout(self.config.shutdown_timeout))
            }
        }
    }

    /// Returns recent stdout/stderr lines for diagnostics.
    #[must_use]
    pub fn recent_logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    async fn wait_ready(&mut self) -> Result<(), SsrError> {
        let deadline = tokio::time::Instant::now() + self.config.startup_timeout;
        loop {
            if let Some(status) = self.take_exit_status() {
                let logs = self.recent_logs().join("\n");
                return Err(SsrError::EarlyExit(if logs.is_empty() {
                    status.to_string()
                } else {
                    format!("{status}: {logs}")
                }));
            }
            if self.probe().await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SsrError::StartupTimeout(self.config.startup_timeout));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn take_exit_status(&mut self) -> Option<ExitStatus> {
        let status = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        if status.is_some() {
            self.child = None;
            self.started_at = None;
        }
        status
    }

    async fn probe(&self) -> Result<(), SsrError> {
        let response = self
            .client
            .get(self.config.health_url())
            .send()
            .await
            .map_err(|error| SsrError::Health(error.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(SsrError::Health(format!(
                "health endpoint returned HTTP {}",
                response.status()
            )))
        }
    }
}

impl Drop for WorkerdProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.start_kill().ok();
        }
    }
}

fn capture_lines<R>(
    reader: R,
    logs: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
    stream: &'static str,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(stream, line = %line, "workerd output");
            if let Ok(mut logs) = logs.lock() {
                if logs.len() == capacity {
                    logs.pop_front();
                }
                logs.push_back(format!("[{stream}] {line}"));
            }
        }
    });
}

#[cfg(unix)]
fn signal_terminate(child: &Child) {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    if let Some(pid) = child.id()
        && let Ok(pid) = i32::try_from(pid)
    {
        kill(Pid::from_raw(pid), Signal::SIGTERM).ok();
    }
}

#[cfg(not(unix))]
fn signal_terminate(child: &Child) {
    let _ = child.start_kill();
}
