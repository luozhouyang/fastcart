//! Server configuration loading and validation.

use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Complete server configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// HTTP listener settings.
    pub server: ServerConfig,
    /// Database locations and SQLite settings.
    pub database: DatabaseConfig,
    /// Asset storage settings.
    pub storage: StorageConfig,
    /// In-process cache settings.
    pub cache: CacheConfig,
    /// Storefront runtime settings.
    pub storefront: StorefrontConfig,
    /// Background job settings.
    pub jobs: JobsConfig,
}

/// HTTP listener settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Bind address.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Default tracing filter.
    pub log_level: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_owned(),
            port: 8080,
            log_level: "info".to_owned(),
        }
    }
}

/// Database settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// Platform SQLite file.
    pub platform_path: PathBuf,
    /// Directory containing per-store SQLite files.
    pub stores_dir: PathBuf,
    /// Journal mode. WAL is the supported initial mode.
    pub journal_mode: String,
    /// SQLite busy timeout in milliseconds.
    pub busy_timeout_ms: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            platform_path: PathBuf::from("./data/platform.sqlite"),
            stores_dir: PathBuf::from("./data/stores"),
            journal_mode: "wal".to_owned(),
            busy_timeout_ms: 5_000,
        }
    }
}

/// Asset storage settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Storage provider identifier.
    pub provider: String,
    /// Local storage root.
    pub root: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            provider: "local".to_owned(),
            root: PathBuf::from("./data/assets"),
        }
    }
}

/// Cache settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Maximum number of cache entries.
    pub max_capacity: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: 10_000,
        }
    }
}

/// Storefront runtime settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StorefrontConfig {
    /// Whether storefront SSR is enabled.
    pub enabled: bool,
    /// Runtime provider: `v8` or `node`.
    pub runtime: String,
    /// Number of runtime workers.
    pub worker_count: usize,
    /// Maximum queued requests per worker pool.
    pub queue_capacity: usize,
    /// SSR request timeout in milliseconds.
    pub request_timeout_ms: u64,
}

impl Default for StorefrontConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            runtime: "v8".to_owned(),
            worker_count: 2,
            queue_capacity: 128,
            request_timeout_ms: 10_000,
        }
    }
}

/// Background job settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct JobsConfig {
    /// Whether background jobs are enabled.
    pub enabled: bool,
    /// Number of job workers.
    pub worker_count: usize,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            worker_count: 2,
        }
    }
}

impl Config {
    /// Loads configuration from `FASTCART_CONFIG` or `config/config.toml`.
    pub fn load() -> Result<Self> {
        Self::load_from(None)
    }

    /// Loads configuration from an explicit path, then applies environment
    /// overrides and validates the result.
    pub fn load_from(path: Option<PathBuf>) -> Result<Self> {
        let path = path
            .or_else(|| env::var_os("FASTCART_CONFIG").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("config/config.toml"));

        let mut config = if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?
        } else {
            Self::default()
        };

        config.apply_env()?;
        config.validate()?;
        Ok(config)
    }

    fn apply_env(&mut self) -> Result<()> {
        if let Ok(value) = env::var("FASTCART_SERVER_HOST") {
            self.server.host = value;
        }
        if let Ok(value) = env::var("FASTCART_SERVER_PORT") {
            self.server.port = value
                .parse()
                .context("FASTCART_SERVER_PORT must be a valid port number")?;
        }
        if let Ok(value) = env::var("FASTCART_LOG_LEVEL") {
            self.server.log_level = value;
        }
        Ok(())
    }

    /// Validates configuration invariants.
    pub fn validate(&self) -> Result<()> {
        if self.storefront.worker_count == 0 {
            bail!("storefront.worker_count must be greater than zero");
        }
        if self.storefront.queue_capacity == 0 {
            bail!("storefront.queue_capacity must be greater than zero");
        }
        if self.storefront.request_timeout_ms == 0 {
            bail!("storefront.request_timeout_ms must be greater than zero");
        }
        if self.jobs.enabled && self.jobs.worker_count == 0 {
            bail!("jobs.worker_count must be greater than zero when jobs are enabled");
        }
        if self.database.busy_timeout_ms == 0 {
            bail!("database.busy_timeout_ms must be greater than zero");
        }
        Ok(())
    }

    /// Returns the configured listener address.
    #[must_use]
    pub fn listen_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn example_config_parses() {
        let config: Config =
            toml::from_str(include_str!("../../../config/config.example.toml")).unwrap();

        config.validate().unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.storefront.worker_count, 2);
    }
}
