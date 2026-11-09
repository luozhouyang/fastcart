//! Workerd runtime configuration.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

/// Configuration for one workerd process.
#[derive(Clone, Debug)]
pub struct WorkerdConfig {
    /// Path to the pinned workerd binary.
    pub binary: PathBuf,
    /// Path to the Cap'n Proto configuration file.
    pub config_path: PathBuf,
    /// Address the workerd HTTP socket listens on.
    pub listen_addr: SocketAddr,
    /// Path used by the readiness probe. Defaults to `/`.
    pub health_path: String,
    /// Maximum time to wait for the process to answer its first health check.
    pub startup_timeout: Duration,
    /// Time allowed for a graceful SIGTERM before a forced kill.
    pub shutdown_timeout: Duration,
    /// Timeout applied to gateway requests.
    pub request_timeout: Duration,
    /// Number of stdout/stderr lines retained for diagnostics.
    pub log_lines: usize,
}

impl WorkerdConfig {
    /// Creates a configuration with conservative defaults.
    #[must_use]
    pub fn new(
        binary: impl Into<PathBuf>,
        config_path: impl Into<PathBuf>,
        listen_addr: SocketAddr,
    ) -> Self {
        Self {
            binary: binary.into(),
            config_path: config_path.into(),
            listen_addr,
            health_path: "/".to_owned(),
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(30),
            log_lines: 200,
        }
    }

    /// Address used by the in-process client. Wildcard binds are probed on
    /// loopback so the gateway never needs to connect to `0.0.0.0`.
    #[must_use]
    pub fn connect_addr(&self) -> SocketAddr {
        if self.listen_addr.ip().is_unspecified() {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.listen_addr.port())
        } else {
            self.listen_addr
        }
    }

    /// Base URL for the gateway.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.connect_addr())
    }

    /// Readiness URL.
    #[must_use]
    pub fn health_url(&self) -> String {
        let path = if self.health_path.starts_with('/') {
            self.health_path.clone()
        } else {
            format!("/{}", self.health_path)
        };
        format!("{}{path}", self.base_url())
    }

    /// Validates paths and limits that would otherwise fail at process start.
    pub fn validate(&self) -> Result<(), crate::SsrError> {
        if !self.binary.is_file() {
            return Err(crate::SsrError::BinaryNotFound(self.binary.clone()));
        }
        if !self.config_path.is_file() {
            return Err(crate::SsrError::ConfigNotFound(self.config_path.clone()));
        }
        if self.startup_timeout.is_zero() {
            return Err(crate::SsrError::InvalidConfig(
                "startup_timeout must be greater than zero".to_owned(),
            ));
        }
        if self.shutdown_timeout.is_zero() {
            return Err(crate::SsrError::InvalidConfig(
                "shutdown_timeout must be greater than zero".to_owned(),
            ));
        }
        if self.log_lines == 0 {
            return Err(crate::SsrError::InvalidConfig(
                "log_lines must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_binds_are_probed_on_loopback() {
        let config = WorkerdConfig::new(
            "/bin/true",
            "/tmp/config.capnp",
            "0.0.0.0:8788".parse().unwrap(),
        );
        assert_eq!(config.connect_addr().to_string(), "127.0.0.1:8788");
        assert_eq!(config.base_url(), "http://127.0.0.1:8788");
        assert_eq!(config.health_url(), "http://127.0.0.1:8788/");
    }

    #[test]
    fn health_path_is_normalized() {
        let mut config = WorkerdConfig::new(
            "/bin/true",
            "/tmp/config.capnp",
            "127.0.0.1:8788".parse().unwrap(),
        );
        config.health_path = "__fastcart/health".to_owned();
        assert_eq!(
            config.health_url(),
            "http://127.0.0.1:8788/__fastcart/health"
        );
    }
}
