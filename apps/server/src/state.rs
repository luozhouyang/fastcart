//! Process-level state shared by bootstrap and the HTTP layer.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fastcart_api::{ApiConfig, AppContext};
use fastcart_email::{
    EmailProviderRegistry, EmailService, ResendConfig, ResendEmailProvider, SmtpConfig,
    SmtpEmailProvider, SmtpSecurity,
};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
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
    /// Optional universal workerd runtime, shut down during graceful shutdown.
    pub universal_ssr: Option<Arc<crate::universal::UniversalSsr>>,
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
        let metrics = Arc::new(fastcart_http::Metrics::new());
        let internal_ssr_credentials = internal_ssr_credentials(&config, metrics.clone()).await?;
        let api_config = ApiConfig {
            service_name: "fastcart".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            cookie_secure: config.server.cookie_secure,
            allowed_origins: config.server.cors_origins.clone(),
            trusted_proxies: config.server.trusted_proxies.clone(),
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
            storefront_streaming_enabled: config.storefront.streaming_enabled,
            storefront_stream_timeout_ms: config.storefront.stream_timeout_ms,
            internal_fetch_secret: config
                .storefront
                .internal_fetch_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
            internal_ssr_token: config
                .ssr
                .internal_token
                .clone()
                .filter(|secret| !secret.is_empty()),
            internal_ssr_credentials: internal_ssr_credentials.clone(),
            capability_secret: config
                .storefront
                .capability_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
            preview_secret: config
                .server
                .preview_secret
                .clone()
                .filter(|secret| !secret.is_empty()),
            preview_frame_ancestors: config.server.preview_frame_ancestors.clone(),
            themes_root: config.storefront.themes_dir.clone(),
            storefront_runtime_kind: config.storefront.runtime.clone(),
            admin_base_url: config
                .server
                .admin_base_url
                .clone()
                .filter(|url| !url.is_empty()),
            storage_root: config.storage.root.clone(),
            max_asset_upload_bytes: config.storage.max_upload_bytes,
            max_upload_bytes: config.inquiries.max_upload_bytes,
            upload_ttl_seconds: config.inquiries.upload_ttl_seconds,
            inquiry_retention_days: config.inquiries.retention_days,
            notification_retention_days: config.notifications.retention_days,
        };
        let universal_ssr = crate::universal::UniversalSsr::initialize(
            &config.ssr,
            metrics.clone(),
            internal_ssr_credentials.clone(),
        )
        .await?;
        let universal_storefront = universal_ssr.as_ref().map(|runtime| {
            Arc::clone(runtime) as Arc<dyn fastcart_api::state::UniversalStorefront>
        });
        let storefront_runtime = build_storefront_runtime(&config.storefront)?;
        if let Some(runtime) = storefront_runtime.as_ref()
            && let Err(error) = runtime.health_check().await
        {
            tracing::warn!(error = %error, "storefront runtime health check failed; rendering will return 503 until it recovers");
        }
        let email = build_email_service(&config.email)?;
        let context = Arc::new(
            AppContext::new(api_config, platform, databases.clone(), cache)
                .with_storefront_runtime(storefront_runtime.clone())
                .with_universal_storefront(universal_storefront)
                .with_metrics(metrics)
                .with_email(email),
        );
        Ok(Self {
            config,
            context,
            databases,
            storefront_runtime,
            universal_ssr,
        })
    }
}

fn build_email_service(config: &crate::config::EmailConfig) -> Result<EmailService> {
    let registry = build_email_registry(config)?;
    let global_registry = registry.clone();
    let base = config.clone();
    Ok(
        EmailService::new(registry).with_store_factory(Arc::new(move |store_id| {
            if !has_store_email_override(store_id) {
                return global_registry.clone();
            }
            let resolved = resolve_store_email_config(&base, store_id);
            match build_email_registry(&resolved) {
                Ok(registry) => registry,
                Err(error) => {
                    tracing::warn!(
                        store_id = %store_id,
                        error = %error,
                        "invalid per-store email configuration; falling back to the global provider"
                    );
                    global_registry.clone()
                }
            }
        })),
    )
}

fn build_email_registry(config: &crate::config::EmailConfig) -> Result<EmailProviderRegistry> {
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
    Ok(service.registry().clone())
}

/// Env names that can override the global email configuration per store.
const STORE_EMAIL_KEYS: &[&str] = &[
    "EMAIL_PROVIDER",
    "EMAIL_FROM",
    "SMTP_HOST",
    "SMTP_PORT",
    "SMTP_USERNAME",
    "SMTP_PASSWORD",
    "SMTP_SECURITY",
    "RESEND_API_KEY",
];

fn has_store_email_override(store_id: &StoreId) -> bool {
    STORE_EMAIL_KEYS
        .iter()
        .any(|key| store_email_env(store_id, key).is_some())
}

fn resolve_store_email_config(
    base: &crate::config::EmailConfig,
    store_id: &StoreId,
) -> crate::config::EmailConfig {
    use crate::config::{ResendSettings, SmtpSettings};

    let mut config = base.clone();
    if let Some(value) = store_email_env(store_id, "EMAIL_PROVIDER") {
        config.provider = value;
    }
    if let Some(value) = store_email_env(store_id, "EMAIL_FROM") {
        config.from = value;
    }
    let smtp = config.smtp.get_or_insert_with(SmtpSettings::default);
    if let Some(value) = store_email_env(store_id, "SMTP_HOST") {
        smtp.host = value;
    }
    if let Some(value) = store_email_env(store_id, "SMTP_PORT")
        && let Ok(port) = value.parse()
    {
        smtp.port = port;
    }
    if let Some(value) = store_email_env(store_id, "SMTP_USERNAME") {
        smtp.username = Some(value);
    }
    if let Some(value) = store_email_env(store_id, "SMTP_PASSWORD") {
        smtp.password = Some(value);
    }
    if let Some(value) = store_email_env(store_id, "SMTP_SECURITY") {
        smtp.security = value;
    }
    let resend = config.resend.get_or_insert_with(ResendSettings::default);
    if let Some(value) = store_email_env(store_id, "RESEND_API_KEY") {
        resend.api_key = value;
    }
    config
}

fn store_email_env(store_id: &StoreId, key: &str) -> Option<String> {
    let normalized: String = store_id
        .as_str()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    std::env::var(format!("FASTCART_STORE_{normalized}_{key}"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Builds store-scoped internal SSR credentials from deployment config.
///
/// Explicit per-deployment tokens are used only when every deployment has one;
/// otherwise the global token remains a single unscoped credential.
async fn internal_ssr_credentials(
    config: &Config,
    metrics: Arc<fastcart_http::Metrics>,
) -> anyhow::Result<fastcart_api::state::InternalSsrCredentials> {
    let global = config
        .ssr
        .internal_token
        .clone()
        .filter(|token| !token.is_empty());
    let tokens = if config.ssr.deployments.is_empty() {
        global
            .map(|token| {
                vec![fastcart_api::state::InternalSsrToken {
                    token,
                    store_id: None,
                    expires_at: None,
                    preview: false,
                }]
            })
            .unwrap_or_default()
    } else if config
        .ssr
        .deployments
        .iter()
        .all(|deployment| deployment.internal_token.is_some())
    {
        config
            .ssr
            .deployments
            .iter()
            .filter_map(|deployment| {
                Some(fastcart_api::state::InternalSsrToken {
                    token: deployment.internal_token.clone()?,
                    store_id: Some(deployment.store_id.clone()),
                    expires_at: None,
                    preview: deployment.preview,
                })
            })
            .collect()
    } else if let Some(token) = global {
        vec![fastcart_api::state::InternalSsrToken {
            token,
            store_id: None,
            expires_at: None,
            preview: false,
        }]
    } else {
        config
            .ssr
            .deployments
            .iter()
            .filter_map(|deployment| {
                Some(fastcart_api::state::InternalSsrToken {
                    token: deployment.internal_token.clone()?,
                    store_id: Some(deployment.store_id.clone()),
                    expires_at: None,
                    preview: deployment.preview,
                })
            })
            .collect()
    };
    let read_only = config.ssr.credentials_read_only();
    let backend =
        fastcart_api::state::FileCredentialBackend::new(config.ssr.credentials_file(), read_only);
    let mode = if read_only {
        fastcart_api::state::CredentialMode::ReadOnly
    } else {
        fastcart_api::state::CredentialMode::ReadWrite
    };
    match fastcart_api::state::InternalSsrCredentials::load_with_metrics(
        std::sync::Arc::new(backend),
        tokens.clone(),
        config.ssr.credentials.seed_from_config,
        mode,
        Some(metrics.clone()),
    )
    .await
    {
        Ok(credentials) => {
            let interval = config.ssr.credentials.refresh_interval_seconds;
            if interval > 0 {
                credentials.spawn_refresh(std::time::Duration::from_secs(interval));
            }
            Ok(credentials)
        }
        Err(error) if !config.ssr.credentials.required => {
            tracing::warn!(error = %error, "SSR credential backend unavailable; using config fallback");
            let mut credentials = fastcart_api::state::InternalSsrCredentials::new(tokens);
            credentials.set_metrics(metrics);
            Ok(credentials)
        }
        Err(error) => Err(anyhow::Error::new(error).context("failed to load SSR credentials")),
    }
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
