//! Process-level state shared by bootstrap and the HTTP layer.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fastcart_api::{ApiConfig, AppContext};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_platform::PlatformStore;

use crate::config::Config;

/// State owned by the server process.
pub struct AppState {
    /// Loaded configuration.
    pub config: Config,
    /// HTTP application context.
    pub context: Arc<AppContext>,
    /// Per-store database manager, closed during graceful shutdown.
    pub databases: Arc<StoreDatabaseManager>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Opens databases and builds all process services.
    pub async fn initialize(config: Config) -> Result<Self> {
        let database_options = DatabaseOptions {
            max_connections: config.database.max_connections,
            busy_timeout_ms: config.database.busy_timeout_ms,
            wal: config.database.journal_mode.eq_ignore_ascii_case("wal"),
        };
        let platform = PlatformStore::open(&config.database.platform_path, &database_options)
            .await
            .context("failed to open the platform database")?;
        let databases = Arc::new(
            StoreDatabaseManager::new(&config.database.stores_dir, database_options)
                .with_max_pools(64),
        );
        let cache = StoreCache::new(
            config.cache.max_capacity,
            Duration::from_secs(config.cache.ttl_seconds),
        );
        let api_config = ApiConfig {
            service_name: "fastcart".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            cookie_secure: config.server.cookie_secure,
            allowed_origins: config.server.cors_origins.clone(),
            max_body_bytes: 2 * 1024 * 1024,
            backup_dir: config.database.backup_dir.clone(),
            manual_payment_secret: config
                .payments
                .webhook_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
        };
        let context = Arc::new(AppContext::new(
            api_config,
            platform,
            databases.clone(),
            cache,
        ));
        Ok(Self {
            config,
            context,
            databases,
        })
    }
}
