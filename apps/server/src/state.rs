//! Process-level state shared by bootstrap and the HTTP layer.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fastcart_api::{ApiConfig, AppContext};
use fastcart_email::{
    EmailService, ResendConfig, ResendEmailProvider, SmtpConfig, SmtpEmailProvider, SmtpSecurity,
};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_platform::PlatformStore;
use fastcart_runtime_api::StorefrontRuntime;
use fastcart_runtime_node::NodeRuntime;
use fastcart_runtime_pool::RuntimePool;

use crate::config::Config;

/// State owned by the server process.
pub struct AppState {
    /// Loaded configuration.
    pub config: Config,
    /// HTTP application context.
    pub context: Arc<AppContext>,
    /// Per-store database manager, closed during graceful shutdown.
    pub databases: Arc<StoreDatabaseManager>,
    /// Optional SSR runtime, shut down during graceful shutdown.
    pub storefront_runtime: Option<Arc<dyn StorefrontRuntime>>,
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
            stripe_secret_key: config
                .payments
                .stripe_secret_key
                .clone()
                .filter(|secret| !secret.is_empty()),
            stripe_webhook_secret: config
                .payments
                .stripe_webhook_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
            storefront_timeout_ms: config.storefront.request_timeout_ms,
            internal_fetch_secret: config
                .storefront
                .internal_fetch_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
        };
        let storefront_runtime = build_storefront_runtime(&config.storefront)?;
        let email = build_email_service(&config.email)?;
        let context = Arc::new(
            AppContext::new(api_config, platform, databases.clone(), cache)
                .with_storefront_runtime(storefront_runtime.clone())
                .with_email(email),
        );
        Ok(Self {
            config,
            context,
            databases,
            storefront_runtime,
        })
    }
}

fn build_email_service(config: &crate::config::EmailConfig) -> Result<EmailService> {
    let service = EmailService::with_default_registry();
    match config.provider.as_str() {
        "log" => {}
        "smtp" => {
            let settings = config
                .smtp
                .as_ref()
                .context("email.smtp is required when email.provider = \"smtp\"")?;
            let security = match settings.security.as_str() {
                "implicit" => SmtpSecurity::ImplicitTls,
                "starttls" => SmtpSecurity::StartTls,
                "plain" => SmtpSecurity::Plain,
                other => anyhow::bail!("unsupported SMTP security mode: {other}"),
            };
            let mut smtp = SmtpConfig::new(&settings.host, settings.port, &config.from)
                .with_security(security);
            if let (Some(username), Some(password)) =
                (settings.username.clone(), settings.password.clone())
            {
                smtp = smtp.with_credentials(username, password);
            }
            let provider = SmtpEmailProvider::new(smtp)
                .map_err(|error| anyhow::anyhow!(error.to_string()))
                .context("failed to configure the SMTP email provider")?;
            service
                .registry()
                .register(Arc::new(provider))
                .context("failed to register the SMTP email provider")?;
            service
                .registry()
                .set_default("smtp")
                .context("failed to select the SMTP email provider")?;
        }
        "resend" => {
            let settings = config
                .resend
                .as_ref()
                .context("email.resend is required when email.provider = \"resend\"")?;
            let provider = ResendEmailProvider::new(ResendConfig {
                api_key: settings.api_key.clone(),
                from: config.from.clone(),
            });
            service
                .registry()
                .register(Arc::new(provider))
                .context("failed to register the Resend email provider")?;
            service
                .registry()
                .set_default("resend")
                .context("failed to select the Resend email provider")?;
        }
        other => anyhow::bail!("unsupported email provider: {other}"),
    }
    Ok(service)
}

fn build_storefront_runtime(
    config: &crate::config::StorefrontConfig,
) -> Result<Option<Arc<dyn StorefrontRuntime>>> {
    if !config.enabled {
        return Ok(None);
    }
    match config.runtime.as_str() {
        "node" => {
            let url = config
                .node_url
                .clone()
                .context("storefront.node_url is required for the node runtime")?;
            let mut workers: Vec<Arc<dyn StorefrontRuntime>> =
                Vec::with_capacity(config.worker_count);
            for _ in 0..config.worker_count.max(1) {
                let mut runtime = NodeRuntime::new(url.clone());
                if let Some(secret) = config.node_secret.clone() {
                    runtime = runtime.with_secret(secret);
                }
                workers.push(Arc::new(runtime));
            }
            let pool = RuntimePool::with_capacity(workers, config.queue_capacity)
                .context("failed to create the storefront runtime pool")?;
            Ok(Some(Arc::new(pool)))
        }
        "v8" => {
            tracing::warn!(
                "the embedded V8 runtime is not available in this build; set storefront.runtime = \"node\" or disable storefront SSR"
            );
            Ok(None)
        }
        other => {
            tracing::warn!(runtime = other, "unknown storefront runtime");
            Ok(None)
        }
    }
}
