//! Generated workerd configuration for one runtime generation.

use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::Duration;

use uuid::Uuid;

use crate::error::PluginError;

const GATEWAY_JS: &str = include_str!("../../assets/gateway.js");
const BRIDGE_JS: &str = include_str!("../../assets/bridge.js");

/// Configuration for the shared plugin host process.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// Pinned workerd binary.
    pub binary: PathBuf,
    /// Directory holding runtime generations.
    pub runtime_dir: PathBuf,
    /// Address of the FastCart HTTP server, reachable from workerd.
    pub commerce_addr: SocketAddr,
    /// Time allowed for the first health probe after spawn.
    pub startup_timeout: Duration,
    /// Time allowed for a graceful SIGTERM before a forced kill.
    pub shutdown_timeout: Duration,
    /// Delay after a timed-out request before the host is restarted.
    pub watchdog_grace: Duration,
    /// Maximum concurrent requests per plugin.
    pub max_concurrency: usize,
    /// Maximum consecutive start failures.
    pub max_restarts: u32,
    /// Number of workerd log lines retained per generation.
    pub log_lines: usize,
    /// Bearer token the gateway uses to fetch bundles.
    pub runtime_token: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("experiments/workerd/bin/workerd"),
            runtime_dir: PathBuf::from("./data/plugins/runtime"),
            commerce_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            watchdog_grace: Duration::from_secs(5),
            max_concurrency: 8,
            max_restarts: 5,
            log_lines: 200,
            runtime_token: "dev-runtime-token".to_owned(),
        }
    }
}

impl RuntimeConfig {
    /// Validates paths and limits before attempting a spawn.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Runtime`] for invalid settings.
    pub fn validate(&self) -> Result<(), PluginError> {
        if !self.binary.is_file() {
            return Err(PluginError::Runtime(format!(
                "workerd binary not found at {}",
                self.binary.display()
            )));
        }
        if self.startup_timeout.is_zero()
            || self.shutdown_timeout.is_zero()
            || self.watchdog_grace.is_zero()
        {
            return Err(PluginError::Runtime(
                "runtime timeouts must be greater than zero".to_owned(),
            ));
        }
        if self.max_concurrency == 0 {
            return Err(PluginError::Runtime(
                "runtime concurrency must be greater than zero".to_owned(),
            ));
        }
        if self.runtime_token.is_empty() {
            return Err(PluginError::Runtime(
                "runtime token must not be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// One generated set of workerd inputs.
#[derive(Clone, Debug)]
pub struct Generation {
    /// Opaque generation identifier.
    pub id: String,
    /// Directory holding `config.capnp`, `gateway.js` and `bridge.js`.
    pub dir: PathBuf,
    /// Port the gateway listens on.
    pub listen_addr: SocketAddr,
}

impl Generation {
    /// Path of the generated Cap'n Proto config.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.capnp")
    }

    /// Health URL.
    #[must_use]
    pub fn health_url(&self) -> String {
        format!("http://{}/__fastcart/health", self.listen_addr)
    }
}

/// Writes a fresh generation and reserves an ephemeral loopback port.
///
/// # Errors
///
/// Returns [`PluginError::Runtime`] when the directory or files cannot be
/// written, or when no loopback port is available.
pub fn prepare_generation(config: &RuntimeConfig) -> Result<Generation, PluginError> {
    let id = format!("gen_{}", Uuid::now_v7().simple());
    let dir = config.runtime_dir.join("generations").join(&id);
    fs::create_dir_all(&dir)
        .map_err(|error| PluginError::Runtime(format!("create runtime dir: {error}")))?;
    write_file(&dir.join("gateway.js"), GATEWAY_JS)?;
    write_file(&dir.join("bridge.js"), BRIDGE_JS)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| PluginError::Runtime(format!("reserve port: {error}")))?;
    let listen_addr = listener
        .local_addr()
        .map_err(|error| PluginError::Runtime(format!("reserve port: {error}")))?;
    drop(listener);

    let capnp = render_config(config.commerce_addr, listen_addr, &config.runtime_token);
    write_file(&dir.join("config.capnp"), &capnp)?;
    Ok(Generation {
        id,
        dir,
        listen_addr,
    })
}

fn write_file(path: &Path, contents: &str) -> Result<(), PluginError> {
    fs::write(path, contents)
        .map_err(|error| PluginError::Runtime(format!("write {}: {error}", path.display())))
}

/// Renders the static Cap'n Proto configuration.
#[must_use]
pub fn render_config(
    commerce_addr: SocketAddr,
    listen_addr: SocketAddr,
    runtime_token: &str,
) -> String {
    format!(
        r#"using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "gateway", worker = .gateway),
    (name = "bridge", worker = .bridge),
    (name = "commerce", external = (address = "{commerce_addr}", http = ())),
  ],
  sockets = [
    (name = "http", address = "{listen_addr}", http = (), service = "gateway"),
  ],
);

const gateway :Workerd.Worker = (
  modules = [(name = "gateway.js", esModule = embed "gateway.js")],
  compatibilityDate = "2026-11-01",
  bindings = [
    (name = "LOADER", workerLoader = (id = "plugins")),
    (name = "BRIDGE", service = "bridge"),
    (name = "COMMERCE", service = "commerce"),
    (name = "RUNTIME_TOKEN", text = "{runtime_token}"),
  ],
);

const bridge :Workerd.Worker = (
  modules = [(name = "bridge.js", esModule = embed "bridge.js")],
  compatibilityDate = "2026-11-01",
  bindings = [(name = "UPSTREAM", service = "commerce")],
);
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_contains_bindings_and_addresses() {
        let rendered = render_config(
            "127.0.0.1:8080".parse().unwrap(),
            "127.0.0.1:9090".parse().unwrap(),
            "token",
        );
        assert!(rendered.contains("external = (address = \"127.0.0.1:8080\""));
        assert!(rendered.contains("address = \"127.0.0.1:9090\""));
        assert!(rendered.contains("workerLoader = (id = \"plugins\")"));
        assert!(rendered.contains("text = \"token\""));
    }

    #[test]
    fn prepares_a_generation_with_assets() {
        let temp = tempfile::tempdir().unwrap();
        let config = RuntimeConfig {
            runtime_dir: temp.path().to_path_buf(),
            binary: temp.path().join("workerd"),
            ..RuntimeConfig::default()
        };
        let generation = prepare_generation(&config).unwrap();
        assert!(generation.config_path().is_file());
        assert!(generation.dir.join("gateway.js").is_file());
        assert!(generation.dir.join("bridge.js").is_file());
        assert_ne!(generation.listen_addr.port(), 0);
    }
}
