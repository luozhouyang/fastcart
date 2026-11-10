//! Supervised workerd process for the shared plugin host.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::Client;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::{sleep, timeout};

use super::config::{Generation, RuntimeConfig, prepare_generation};
use crate::error::PluginError;

/// Request class, which selects the deadline and response cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    /// Admin RPC: 10 s, 512 KB.
    Admin,
    /// Storefront route: 30 s, 8 MB.
    Storefront,
    /// Event delivery: 30 s, 8 MB.
    Event,
    /// Scheduled job: 60 s, 8 MB.
    Job,
}

impl RequestKind {
    /// Deadline for one plugin request.
    #[must_use]
    pub fn deadline(self) -> Duration {
        match self {
            Self::Admin => Duration::from_secs(10),
            Self::Storefront | Self::Event => Duration::from_secs(30),
            Self::Job => Duration::from_secs(60),
        }
    }

    /// Maximum accepted response body.
    #[must_use]
    pub fn max_response_bytes(self) -> usize {
        match self {
            Self::Admin => 512 * 1024,
            Self::Storefront | Self::Event | Self::Job => 8 * 1024 * 1024,
        }
    }
}

/// A request forwarded to the plugin gateway.
#[derive(Clone, Debug)]
pub struct RuntimeRequest {
    /// Plugin id.
    pub plugin_id: String,
    /// Artifact digest.
    pub digest: String,
    /// Store id, bound to the context token.
    pub store_id: String,
    /// Path and query below the plugin root.
    pub path_and_query: String,
    /// HTTP method.
    pub method: String,
    /// Request headers already filtered by the host.
    pub headers: Vec<(String, String)>,
    /// Short-lived context token.
    pub context_token: String,
    /// Request body.
    pub body: Vec<u8>,
    /// Request class.
    pub kind: RequestKind,
    /// Test or tuning override for the request deadline.
    pub deadline_override: Option<Duration>,
}

/// A response returned by the plugin gateway.
#[derive(Clone, Debug)]
pub struct RuntimeResponse {
    /// HTTP status.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
}

/// A streaming response from the plugin gateway.
pub struct StreamingResponse {
    /// HTTP status.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Streamed body with a hard size cap.
    pub stream: BoxStream<'static, Result<Bytes, std::io::Error>>,
}

/// Runtime health snapshot.
#[derive(Clone, Debug)]
pub struct RuntimeHealth {
    /// Whether workerd is running and passed its last probe.
    pub running: bool,
    /// Current generation id.
    pub generation: Option<String>,
    /// Loopback port of the gateway.
    pub port: Option<u16>,
    /// Number of watchdog/crash restarts.
    pub restarts: u64,
    /// Last error, when unhealthy.
    pub last_error: Option<String>,
    /// Uptime in seconds.
    pub uptime_seconds: u64,
}

struct HostState {
    child: Option<Child>,
    generation: Option<Generation>,
    started_at: Option<Instant>,
    restarts: u64,
    consecutive_failures: u32,
    last_error: Option<String>,
}

/// The shared workerd process and its per-plugin admission control.
pub struct PluginHost {
    config: RuntimeConfig,
    client: Client,
    state: Mutex<HostState>,
    semaphores: std::sync::Mutex<HashMap<String, Arc<Semaphore>>>,
    restarts: AtomicU64,
    timeouts: AtomicU64,
    requests: AtomicU64,
}

impl PluginHost {
    /// Creates a host that starts workerd on first use.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Runtime`] when the client cannot be built.
    pub fn new(config: RuntimeConfig) -> Result<Self, PluginError> {
        config.validate()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| PluginError::Runtime(error.to_string()))?;
        Ok(Self {
            config,
            client,
            state: Mutex::new(HostState {
                child: None,
                generation: None,
                started_at: None,
                restarts: 0,
                consecutive_failures: 0,
                last_error: None,
            }),
            semaphores: std::sync::Mutex::new(HashMap::new()),
            restarts: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            requests: AtomicU64::new(0),
        })
    }

    /// Returns the runtime configuration.
    #[must_use]
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Forwards a request and buffers the response.
    ///
    /// # Errors
    ///
    /// Returns timeout, admission-control, process and HTTP errors.
    pub async fn forward(
        self: &Arc<Self>,
        request: RuntimeRequest,
    ) -> Result<RuntimeResponse, PluginError> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let (builder, generation, permit) = self.prepare(&request).await?;
        let (response, _permit) = self.send(&request, builder, generation, permit).await?;
        let status = response.status().as_u16();
        let headers = response_headers(&response);
        let body = response
            .bytes()
            .await
            .map_err(|error| PluginError::Runtime(format!("read plugin response: {error}")))?;
        if body.len() > request.kind.max_response_bytes() {
            return Err(PluginError::Runtime(format!(
                "plugin response exceeds {} bytes",
                request.kind.max_response_bytes()
            )));
        }
        Ok(RuntimeResponse {
            status,
            headers,
            body: body.to_vec(),
        })
    }

    /// Forwards a request and streams the response body.
    ///
    /// The deadline covers the response head; the body stream keeps the
    /// per-plugin permit and enforces the response cap while it is read.
    ///
    /// # Errors
    ///
    /// Returns timeout, admission-control, process and HTTP errors.
    pub async fn forward_stream(
        self: &Arc<Self>,
        request: RuntimeRequest,
    ) -> Result<StreamingResponse, PluginError> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let (builder, generation, permit) = self.prepare(&request).await?;
        let (response, permit) = self.send(&request, builder, generation, permit).await?;
        let status = response.status().as_u16();
        let headers = response_headers(&response);
        let max = request.kind.max_response_bytes();
        let stream = futures::stream::unfold(
            (response, permit, 0_usize, false),
            move |(mut response, permit, total, done)| async move {
                if done {
                    return None;
                }
                match response.chunk().await {
                    Ok(Some(bytes)) => {
                        let next = total.saturating_add(bytes.len());
                        if next > max {
                            Some((
                                Err(std::io::Error::other(format!(
                                    "plugin response exceeds {max} bytes"
                                ))),
                                (response, permit, next, true),
                            ))
                        } else {
                            Some((Ok(bytes), (response, permit, next, false)))
                        }
                    }
                    Ok(None) => None,
                    Err(error) => Some((
                        Err(std::io::Error::other(error.to_string())),
                        (response, permit, total, true),
                    )),
                }
            },
        )
        .boxed();
        Ok(StreamingResponse {
            status,
            headers,
            stream,
        })
    }

    async fn prepare(
        &self,
        request: &RuntimeRequest,
    ) -> Result<(reqwest::RequestBuilder, Generation, OwnedSemaphorePermit), PluginError> {
        let semaphore = self.semaphore(&request.plugin_id);
        let permit = timeout(Duration::from_secs(30), semaphore.acquire_owned())
            .await
            .map_err(|_| PluginError::Runtime("plugin request queue timed out".to_owned()))?
            .map_err(|_| PluginError::Runtime("plugin runtime is shutting down".to_owned()))?;
        let generation = self.ensure_started().await?;
        let url = format!(
            "http://{}/__plugin/{}@{}{}",
            generation.listen_addr,
            request.plugin_id,
            request.digest,
            normalize_path(&request.path_and_query)
        );
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &request.headers {
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }
        headers.insert(
            "x-fastcart-store-id",
            reqwest::header::HeaderValue::from_str(&request.store_id)
                .map_err(|_| PluginError::Runtime("invalid store id".to_owned()))?,
        );
        headers.insert(
            "x-fastcart-context-token",
            reqwest::header::HeaderValue::from_str(&request.context_token)
                .map_err(|_| PluginError::Runtime("invalid context token".to_owned()))?,
        );
        headers.insert(
            "x-fastcart-gateway-token",
            reqwest::header::HeaderValue::from_str(&self.config.runtime_token)
                .map_err(|_| PluginError::Runtime("invalid runtime token".to_owned()))?,
        );
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| PluginError::Runtime("invalid method".to_owned()))?;
        let mut builder = self.client.request(method, &url).headers(headers);
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        Ok((builder, generation, permit))
    }

    async fn send(
        self: &Arc<Self>,
        request: &RuntimeRequest,
        builder: reqwest::RequestBuilder,
        generation: Generation,
        permit: OwnedSemaphorePermit,
    ) -> Result<(reqwest::Response, OwnedSemaphorePermit), PluginError> {
        let generation_id = generation.id.clone();
        let deadline = request
            .deadline_override
            .unwrap_or_else(|| request.kind.deadline());
        match timeout(deadline, builder.send()).await {
            Ok(Ok(response)) => Ok((response, permit)),
            Ok(Err(error)) => Err(PluginError::Runtime(format!(
                "gateway request failed: {error}"
            ))),
            Err(_) => {
                self.timeouts.fetch_add(1, Ordering::Relaxed);
                self.schedule_watchdog(generation_id);
                Err(PluginError::Runtime("plugin request timed out".to_owned()))
            }
        }
    }

    /// Ensures the host is running and returns the gateway address.
    ///
    /// # Errors
    ///
    /// Returns the startup error when workerd cannot be reached.
    pub async fn gateway_addr(&self) -> Result<SocketAddr, PluginError> {
        let generation = self.ensure_started().await?;
        Ok(generation.listen_addr)
    }

    /// Returns runtime health after probing the process.
    pub async fn health(&self) -> RuntimeHealth {
        let mut state = self.state.lock().await;
        let running = state
            .child
            .as_mut()
            .map(|child| matches!(child.try_wait(), Ok(None)))
            .unwrap_or(false);
        if running
            && let Some(generation) = &state.generation
            && self.probe(generation).await.is_err()
        {
            state.last_error = Some("health probe failed".to_owned());
        }
        RuntimeHealth {
            running,
            generation: state
                .generation
                .as_ref()
                .map(|generation| generation.id.clone()),
            port: state
                .generation
                .as_ref()
                .map(|generation| generation.listen_addr.port()),
            restarts: self.restarts.load(Ordering::Relaxed),
            last_error: state.last_error.clone(),
            uptime_seconds: state
                .started_at
                .map_or(0, |started| started.elapsed().as_secs()),
        }
    }

    /// Stops the host process.
    pub async fn shutdown(&self) {
        let mut state = self.state.lock().await;
        stop_child(&mut state, self.config.shutdown_timeout).await;
        state.generation = None;
        state.started_at = None;
    }

    fn semaphore(&self, plugin_id: &str) -> Arc<Semaphore> {
        let mut semaphores = self.semaphores.lock().expect("semaphore map poisoned");
        Arc::clone(
            semaphores
                .entry(plugin_id.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(self.config.max_concurrency))),
        )
    }

    async fn ensure_started(&self) -> Result<Generation, PluginError> {
        let mut state = self.state.lock().await;
        if state
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
        {
            return state
                .generation
                .clone()
                .ok_or_else(|| PluginError::Runtime("runtime generation missing".to_owned()));
        }
        if state.consecutive_failures >= self.config.max_restarts {
            return Err(PluginError::Runtime(
                "plugin runtime restart limit reached".to_owned(),
            ));
        }
        if state.consecutive_failures > 0 {
            let backoff = Duration::from_secs(1 << state.consecutive_failures.min(3));
            sleep(backoff).await;
        }
        start_child(&self.config, &self.client, &mut state).await?;
        state.consecutive_failures = 0;
        state
            .generation
            .clone()
            .ok_or_else(|| PluginError::Runtime("runtime generation missing".to_owned()))
    }

    fn schedule_watchdog(self: &Arc<Self>, generation_id: String) {
        let host = Arc::clone(self);
        let grace = self.config.watchdog_grace;
        tokio::spawn(async move {
            sleep(grace).await;
            let mut state = host.state.lock().await;
            let current = state
                .generation
                .as_ref()
                .is_some_and(|generation| generation.id == generation_id);
            if !current {
                return;
            }
            state.last_error = Some("watchdog restarted an unresponsive plugin host".to_owned());
            stop_child(&mut state, host.config.shutdown_timeout).await;
            state.child = None;
            state.generation = None;
            state.restarts += 1;
            host.restarts.fetch_add(1, Ordering::Relaxed);
            if let Err(error) = start_child(&host.config, &host.client, &mut state).await {
                state.last_error = Some(error.to_string());
                state.consecutive_failures += 1;
                tracing::error!(error = %error, "plugin host watchdog restart failed");
            }
        });
    }

    async fn probe(&self, generation: &Generation) -> Result<(), PluginError> {
        let response = timeout(
            Duration::from_secs(2),
            self.client.get(generation.health_url()).send(),
        )
        .await
        .map_err(|_| PluginError::Runtime("health probe timed out".to_owned()))?
        .map_err(|error| PluginError::Runtime(error.to_string()))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(PluginError::Runtime(format!(
                "health probe returned {}",
                response.status()
            )))
        }
    }
}

async fn start_child(
    config: &RuntimeConfig,
    client: &Client,
    state: &mut HostState,
) -> Result<(), PluginError> {
    let generation = prepare_generation(config)?;
    let mut command = Command::new(&config.binary);
    command
        .arg("serve")
        .arg("--experimental")
        .arg(generation.config_path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|error| PluginError::Runtime(format!("spawn workerd: {error}")))?;
    if let Some(stdout) = child.stdout.take() {
        drain_logs(stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        drain_logs(stderr);
    }

    let deadline = Instant::now() + config.startup_timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(PluginError::Runtime(format!(
                "workerd exited during startup: {status}"
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err(PluginError::Runtime(
                "workerd did not become ready before the startup deadline".to_owned(),
            ));
        }
        match timeout(
            Duration::from_millis(500),
            client.get(generation.health_url()).send(),
        )
        .await
        {
            Ok(Ok(response)) if response.status().is_success() => break,
            _ => sleep(Duration::from_millis(100)).await,
        }
    }

    state.child = Some(child);
    state.generation = Some(generation);
    state.started_at = Some(Instant::now());
    state.last_error = None;
    Ok(())
}

async fn stop_child(state: &mut HostState, shutdown_timeout: Duration) {
    let Some(mut child) = state.child.take() else {
        return;
    };
    let _ = child.start_kill();
    let _ = timeout(shutdown_timeout, child.wait()).await;
}

fn drain_logs<R>(reader: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "fastcart_plugin_workerd", "{line}");
        }
    });
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn response_headers(response: &reqwest::Response) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .filter(|(name, _)| !is_hop_by_hop(name.as_str()) && name.as_str() != "set-cookie")
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}
