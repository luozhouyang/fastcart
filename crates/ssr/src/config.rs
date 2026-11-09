//! Universal SSR runtime settings.
//!
//! These types are deserialized from the `[ssr]` table and its
//! `FASTCART_SSR_*` environment overrides. They describe one deployment
//! topology; they are not process state.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Universal workerd SSR settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SsrConfig {
    /// Whether the universal runtime is enabled.
    pub enabled: bool,
    /// Pinned workerd binary.
    pub binary: PathBuf,
    /// Source artifact directory (`fastcart-ssr.json` at its root).
    pub artifact_path: PathBuf,
    /// Directory for generated configs and imported artifacts.
    pub runtime_dir: PathBuf,
    /// Loopback address assigned to the runtime group.
    pub listen: SocketAddr,
    /// Deployment identity injected into SSR requests.
    pub deployment_id: String,
    /// Optional internal token injected into SSR requests.
    pub internal_token: Option<String>,
    /// Readiness timeout in milliseconds.
    pub startup_timeout_ms: u64,
    /// Graceful shutdown timeout in milliseconds.
    pub shutdown_timeout_ms: u64,
    /// Gateway request timeout in milliseconds.
    pub request_timeout_ms: u64,
    /// Minimum restart delay in milliseconds.
    pub restart_backoff_ms: u64,
    /// Maximum restarts before the group fails.
    pub max_restarts: u32,
    /// Time to keep a replaced deployment running for in-flight requests.
    pub drain_timeout_ms: u64,
    /// Maximum artifact size in bytes.
    pub artifact_max_bytes: u64,
    /// Maximum artifact file count.
    pub artifact_max_files: u64,
    /// Optional root that runtime activation paths must stay under.
    pub artifact_root: Option<PathBuf>,
    /// Optional JSON file for persisted internal SSR credentials.
    ///
    /// Defaults to `<runtime_dir>/internal-ssr-credentials.json`.
    pub credentials_path: Option<PathBuf>,
    /// Credential backend settings.
    pub credentials: SsrCredentialsConfig,
    /// Explicit per-store deployments. Empty starts one default deployment.
    pub deployments: Vec<SsrDeploymentConfig>,
}

/// Credential backend settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SsrCredentialsConfig {
    /// Backend identifier; only `file` is implemented.
    pub backend: String,
    /// `read_write` or `read_only`.
    pub mode: String,
    /// Optional file override; falls back to `ssr.credentials_path`.
    pub path: Option<PathBuf>,
    /// Background refresh interval; `0` disables polling.
    pub refresh_interval_seconds: u64,
    /// Fail startup when the backend cannot be read.
    pub required: bool,
    /// Seed an empty backend from configured deployment tokens.
    pub seed_from_config: bool,
    /// File encryption: `none` or `xchacha20poly1305`.
    pub encryption: String,
    /// Environment variable holding the base64 encryption key.
    pub key_env: String,
}

impl Default for SsrCredentialsConfig {
    fn default() -> Self {
        Self {
            backend: "file".to_owned(),
            mode: "read_write".to_owned(),
            path: None,
            refresh_interval_seconds: 30,
            required: true,
            seed_from_config: true,
            encryption: "none".to_owned(),
            key_env: "FASTCART_SSR_CREDENTIALS_KEY".to_owned(),
        }
    }
}

/// One store deployment served by the universal runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SsrDeploymentConfig {
    /// Store served by this deployment.
    pub store_id: String,
    /// Deployment identity injected into SSR requests.
    pub deployment_id: String,
    /// Source artifact directory.
    pub artifact_path: PathBuf,
    /// Optional loopback listener; defaults to the next port after `ssr.listen`.
    pub listen: Option<SocketAddr>,
    /// Marks this deployment as a preview target for the store.
    pub preview: bool,
    /// Optional deployment-scoped internal token.
    pub internal_token: Option<String>,
}

impl Default for SsrDeploymentConfig {
    fn default() -> Self {
        Self {
            store_id: String::new(),
            deployment_id: String::new(),
            artifact_path: PathBuf::new(),
            listen: None,
            preview: false,
            internal_token: None,
        }
    }
}

impl Default for SsrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            binary: PathBuf::from("./workerd"),
            artifact_path: PathBuf::from("./storefront-artifact"),
            runtime_dir: PathBuf::from("./data/ssr"),
            listen: SocketAddr::from(([127, 0, 0, 1], 18_888)),
            deployment_id: "default".to_owned(),
            internal_token: None,
            startup_timeout_ms: 10_000,
            shutdown_timeout_ms: 5_000,
            request_timeout_ms: 30_000,
            restart_backoff_ms: 1_000,
            max_restarts: 3,
            drain_timeout_ms: 5_000,
            artifact_max_bytes: 100 * 1024 * 1024,
            artifact_max_files: 5_000,
            artifact_root: None,
            credentials_path: None,
            credentials: SsrCredentialsConfig::default(),
            deployments: Vec::new(),
        }
    }
}

impl SsrConfig {
    /// Resolved credential file path.
    #[must_use]
    pub fn credentials_file(&self) -> PathBuf {
        self.credentials
            .path
            .clone()
            .or_else(|| self.credentials_path.clone())
            .unwrap_or_else(|| self.runtime_dir.join("internal-ssr-credentials.json"))
    }

    /// Whether credential writes are disabled.
    #[must_use]
    pub fn credentials_read_only(&self) -> bool {
        self.credentials.mode == "read_only"
    }
}
