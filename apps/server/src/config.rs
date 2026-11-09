//! Server configuration loading and validation.

use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub use fastcart_ssr::{SsrConfig, SsrCredentialsConfig, SsrDeploymentConfig};

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
    /// Universal workerd SSR settings.
    pub ssr: SsrConfig,
    /// Background job settings.
    pub jobs: JobsConfig,
    /// Payment provider settings.
    pub payments: PaymentsConfig,
    /// Transactional email settings.
    pub email: EmailConfig,
    /// Inquiry attachment settings.
    pub inquiries: InquiryConfig,
    /// Notification delivery retention settings.
    pub notifications: NotificationConfig,
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
    /// Whether session cookies carry the Secure attribute.
    pub cookie_secure: bool,
    /// Exact browser origins permitted to call the API with credentials.
    pub cors_origins: Vec<String>,
    /// Exact addresses or CIDRs whose `X-Forwarded-For` chain is trusted.
    pub trusted_proxies: Vec<String>,
    /// Optional signing secret enabling draft page preview.
    ///
    /// When absent, preview token issuance and the `/preview` render surface
    /// respond with `501 Not Implemented`.
    pub preview_secret: Option<String>,
    /// Origins permitted to embed `/preview` in an iframe.
    ///
    /// Empty leaves framing unrestricted for the token-bearing preview path.
    /// Set this to the admin application origin in production.
    pub preview_frame_ancestors: Vec<String>,
    /// Optional admin application origin used for notification deep links.
    pub admin_base_url: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_owned(),
            port: 8080,
            log_level: "info".to_owned(),
            cookie_secure: true,
            cors_origins: Vec::new(),
            trusted_proxies: Vec::new(),
            preview_secret: None,
            preview_frame_ancestors: Vec::new(),
            admin_base_url: None,
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
    /// Maximum connections per store pool.
    pub max_connections: u32,
    /// Directory for on-demand store backups.
    pub backup_dir: PathBuf,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            platform_path: PathBuf::from("./data/platform.sqlite"),
            stores_dir: PathBuf::from("./data/stores"),
            journal_mode: "wal".to_owned(),
            busy_timeout_ms: 5_000,
            max_connections: 5,
            backup_dir: PathBuf::from("./data/backups"),
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
    /// Hard cap for one media-library asset upload.
    pub max_upload_bytes: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            provider: "local".to_owned(),
            root: PathBuf::from("./data/assets"),
            max_upload_bytes: 100 * 1024 * 1024,
        }
    }
}

/// Inquiry attachment settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct InquiryConfig {
    /// Hard cap for one uploaded attachment.
    pub max_upload_bytes: usize,
    /// Lifetime of a pending upload before cleanup.
    pub upload_ttl_seconds: i64,
    /// Days an inquiry and its attachments are retained.
    pub retention_days: i64,
}

impl Default for InquiryConfig {
    fn default() -> Self {
        Self {
            max_upload_bytes: 25 * 1024 * 1024,
            upload_ttl_seconds: 3_600,
            retention_days: 365,
        }
    }
}

/// Notification delivery retention settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    /// Days terminal channel delivery records are retained.
    pub retention_days: i64,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self { retention_days: 90 }
    }
}

/// Cache settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Maximum number of cache entries.
    pub max_capacity: u64,
    /// Entry time to live in seconds.
    pub ttl_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: 10_000,
            ttl_seconds: 60,
        }
    }
}

/// Payment provider settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaymentsConfig {
    /// Shared secret used to verify manual/sandbox webhook signatures.
    pub webhook_secret: Option<String>,
    /// Stripe secret API key.
    pub stripe_secret_key: Option<String>,
    /// Stripe webhook signing secret.
    pub stripe_webhook_secret: Option<String>,
}

/// Transactional email settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    /// Provider: `log`, `smtp` or `resend`.
    pub provider: String,
    /// Default sender address.
    pub from: String,
    /// SMTP settings used when `provider = "smtp"`.
    pub smtp: Option<SmtpSettings>,
    /// Resend settings used when `provider = "resend"`.
    pub resend: Option<ResendSettings>,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            provider: "log".to_owned(),
            from: "no-reply@example.com".to_owned(),
            smtp: None,
            resend: None,
        }
    }
}

/// SMTP connection settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SmtpSettings {
    /// SMTP host.
    pub host: String,
    /// SMTP port.
    pub port: u16,
    /// Optional username.
    pub username: Option<String>,
    /// Optional password.
    pub password: Option<String>,
    /// `implicit`, `starttls` or `plain`.
    pub security: String,
}

impl Default for SmtpSettings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            username: None,
            password: None,
            security: "starttls".to_owned(),
        }
    }
}

/// Resend API settings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResendSettings {
    /// Resend API key.
    pub api_key: String,
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
        let cookie_secure = env::var("FASTCART_COOKIE_SECURE").ok();
        let cors_origins = env::var("FASTCART_CORS_ORIGINS").ok();
        if let Ok(value) = env::var("FASTCART_TRUSTED_PROXIES") {
            self.server.trusted_proxies = value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect();
        }
        self.apply_server_env(cookie_secure.as_deref(), cors_origins.as_deref())?;
        if let Ok(value) = env::var("FASTCART_PAYMENT_WEBHOOK_SECRET") {
            self.payments.webhook_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STRIPE_SECRET_KEY") {
            self.payments.stripe_secret_key = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STRIPE_WEBHOOK_SECRET") {
            self.payments.stripe_webhook_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_SSR_ENABLED") {
            self.ssr.enabled = value
                .parse()
                .context("FASTCART_SSR_ENABLED must be true or false")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_BINARY") {
            self.ssr.binary = PathBuf::from(value);
        }
        if let Ok(value) = env::var("FASTCART_SSR_ARTIFACT") {
            self.ssr.artifact_path = PathBuf::from(value);
        }
        if let Ok(value) = env::var("FASTCART_SSR_RUNTIME_DIR") {
            self.ssr.runtime_dir = PathBuf::from(value);
        }
        if let Ok(value) = env::var("FASTCART_SSR_LISTEN") {
            self.ssr.listen = value
                .parse()
                .context("FASTCART_SSR_LISTEN must be a socket address")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_DEPLOYMENT_ID") {
            self.ssr.deployment_id = value;
        }
        if let Ok(value) = env::var("FASTCART_SSR_INTERNAL_TOKEN") {
            self.ssr.internal_token = (!value.trim().is_empty()).then_some(value);
        }
        if let Ok(value) = env::var("FASTCART_SSR_STARTUP_TIMEOUT_MS") {
            self.ssr.startup_timeout_ms = value
                .parse()
                .context("FASTCART_SSR_STARTUP_TIMEOUT_MS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_SHUTDOWN_TIMEOUT_MS") {
            self.ssr.shutdown_timeout_ms = value
                .parse()
                .context("FASTCART_SSR_SHUTDOWN_TIMEOUT_MS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_REQUEST_TIMEOUT_MS") {
            self.ssr.request_timeout_ms = value
                .parse()
                .context("FASTCART_SSR_REQUEST_TIMEOUT_MS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_RESTART_BACKOFF_MS") {
            self.ssr.restart_backoff_ms = value
                .parse()
                .context("FASTCART_SSR_RESTART_BACKOFF_MS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_MAX_RESTARTS") {
            self.ssr.max_restarts = value
                .parse()
                .context("FASTCART_SSR_MAX_RESTARTS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_DRAIN_TIMEOUT_MS") {
            self.ssr.drain_timeout_ms = value
                .parse()
                .context("FASTCART_SSR_DRAIN_TIMEOUT_MS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_ARTIFACT_MAX_BYTES") {
            self.ssr.artifact_max_bytes = value
                .parse()
                .context("FASTCART_SSR_ARTIFACT_MAX_BYTES must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_ARTIFACT_MAX_FILES") {
            self.ssr.artifact_max_files = value
                .parse()
                .context("FASTCART_SSR_ARTIFACT_MAX_FILES must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_ARTIFACT_ROOT") {
            self.ssr.artifact_root = Some(PathBuf::from(value));
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_PATH") {
            self.ssr.credentials_path = Some(PathBuf::from(value));
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_BACKEND") {
            self.ssr.credentials.backend = value;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_MODE") {
            self.ssr.credentials.mode = value;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_FILE") {
            self.ssr.credentials.path = Some(PathBuf::from(value));
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_REFRESH_SECONDS") {
            self.ssr.credentials.refresh_interval_seconds = value
                .parse()
                .context("FASTCART_SSR_CREDENTIALS_REFRESH_SECONDS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_REQUIRED") {
            self.ssr.credentials.required = value
                .parse()
                .context("FASTCART_SSR_CREDENTIALS_REQUIRED must be true or false")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_SEED") {
            self.ssr.credentials.seed_from_config = value
                .parse()
                .context("FASTCART_SSR_CREDENTIALS_SEED must be true or false")?;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_ENCRYPTION") {
            self.ssr.credentials.encryption = value;
        }
        if let Ok(value) = env::var("FASTCART_SSR_CREDENTIALS_KEY_ENV") {
            self.ssr.credentials.key_env = value;
        }
        if let Ok(value) = env::var("FASTCART_PREVIEW_SECRET") {
            // An empty pass-through value from an orchestrator
            // disables preview rather than configuring a zero-byte secret.
            self.server.preview_secret = (!value.trim().is_empty()).then_some(value);
        }
        if let Ok(value) = env::var("FASTCART_PREVIEW_FRAME_ANCESTORS") {
            self.server.preview_frame_ancestors = value
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Ok(value) = env::var("FASTCART_ADMIN_BASE_URL") {
            self.server.admin_base_url = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_ASSET_MAX_UPLOAD_BYTES") {
            self.storage.max_upload_bytes = value
                .parse()
                .context("FASTCART_ASSET_MAX_UPLOAD_BYTES must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_INQUIRY_MAX_UPLOAD_BYTES") {
            self.inquiries.max_upload_bytes = value
                .parse()
                .context("FASTCART_INQUIRY_MAX_UPLOAD_BYTES must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_INQUIRY_UPLOAD_TTL_SECONDS") {
            self.inquiries.upload_ttl_seconds = value
                .parse()
                .context("FASTCART_INQUIRY_UPLOAD_TTL_SECONDS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_INQUIRY_RETENTION_DAYS") {
            self.inquiries.retention_days = value
                .parse()
                .context("FASTCART_INQUIRY_RETENTION_DAYS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_NOTIFICATION_RETENTION_DAYS") {
            self.notifications.retention_days = value
                .parse()
                .context("FASTCART_NOTIFICATION_RETENTION_DAYS must be a number")?;
        }
        if let Ok(value) = env::var("FASTCART_EMAIL_PROVIDER") {
            self.email.provider = value;
        }
        if let Ok(value) = env::var("FASTCART_EMAIL_FROM") {
            self.email.from = value;
        }
        if let Ok(value) = env::var("FASTCART_SMTP_HOST") {
            self.email
                .smtp
                .get_or_insert_with(SmtpSettings::default)
                .host = value;
        }
        if let Ok(value) = env::var("FASTCART_SMTP_PORT") {
            self.email
                .smtp
                .get_or_insert_with(SmtpSettings::default)
                .port = value
                .parse()
                .context("FASTCART_SMTP_PORT must be a valid port number")?;
        }
        if let Ok(value) = env::var("FASTCART_SMTP_USERNAME") {
            self.email
                .smtp
                .get_or_insert_with(SmtpSettings::default)
                .username = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_SMTP_PASSWORD") {
            self.email
                .smtp
                .get_or_insert_with(SmtpSettings::default)
                .password = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_SMTP_SECURITY") {
            self.email
                .smtp
                .get_or_insert_with(SmtpSettings::default)
                .security = value;
        }
        if let Ok(value) = env::var("FASTCART_RESEND_API_KEY") {
            self.email
                .resend
                .get_or_insert_with(ResendSettings::default)
                .api_key = value;
        }
        Ok(())
    }

    /// Applies the listener environment overrides that are parsed with helpers.
    fn apply_server_env(
        &mut self,
        cookie_secure: Option<&str>,
        cors_origins: Option<&str>,
    ) -> Result<()> {
        if let Some(value) = cookie_secure {
            self.server.cookie_secure = parse_env_bool(value, "FASTCART_COOKIE_SECURE")?;
        }
        if let Some(value) = cors_origins {
            self.server.cors_origins = parse_origin_list(value, "FASTCART_CORS_ORIGINS")?;
        }
        Ok(())
    }

    /// Validates configuration invariants.
    pub fn validate(&self) -> Result<()> {
        if self.ssr.enabled {
            if self.ssr.binary.as_os_str().is_empty() {
                bail!("ssr.binary must not be empty");
            }
            if self.ssr.runtime_dir.as_os_str().is_empty() {
                bail!("ssr.runtime_dir must not be empty");
            }
            if self.ssr.listen.port() == 0 {
                bail!("ssr.listen must include a non-zero port");
            }
            if !self.ssr.listen.ip().is_loopback() {
                bail!("ssr.listen must bind to loopback in this phase");
            }
            if self.ssr.startup_timeout_ms == 0
                || self.ssr.shutdown_timeout_ms == 0
                || self.ssr.request_timeout_ms == 0
            {
                bail!("ssr timeouts must be greater than zero");
            }
            if self.ssr.artifact_max_bytes == 0 || self.ssr.artifact_max_files == 0 {
                bail!("ssr artifact limits must be greater than zero");
            }
            if self
                .ssr
                .artifact_root
                .as_ref()
                .is_some_and(|root| root.as_os_str().is_empty())
            {
                bail!("ssr.artifact_root must not be empty when set");
            }
            if self
                .ssr
                .credentials_path
                .as_ref()
                .is_some_and(|path| path.as_os_str().is_empty())
            {
                bail!("ssr.credentials_path must not be empty when set");
            }
            if !matches!(self.ssr.credentials.backend.as_str(), "file") {
                bail!("ssr.credentials.backend must be \"file\"");
            }
            if !matches!(
                self.ssr.credentials.mode.as_str(),
                "read_write" | "read_only"
            ) {
                bail!("ssr.credentials.mode must be read_write or read_only");
            }
            if !matches!(
                self.ssr.credentials.encryption.as_str(),
                "none" | "xchacha20poly1305"
            ) {
                bail!("ssr.credentials.encryption must be none or xchacha20poly1305");
            }
            if self.ssr.credentials.key_env.trim().is_empty() {
                bail!("ssr.credentials.key_env must not be empty");
            }
            if self
                .ssr
                .credentials
                .path
                .as_ref()
                .is_some_and(|path| path.as_os_str().is_empty())
            {
                bail!("ssr.credentials.path must not be empty when set");
            }
            if self.ssr.deployments.is_empty() {
                if self.ssr.artifact_path.as_os_str().is_empty() {
                    bail!("ssr.artifact_path must not be empty");
                }
                if !is_safe_identifier(&self.ssr.deployment_id) {
                    bail!("ssr.deployment_id must be 1-128 safe characters");
                }
            } else {
                let mut stores: Vec<(String, bool)> = Vec::new();
                let mut ports = Vec::new();
                for (index, deployment) in self.ssr.deployments.iter().enumerate() {
                    if !is_safe_identifier(&deployment.store_id) {
                        bail!("ssr.deployments[{index}].store_id must be 1-128 safe characters");
                    }
                    if !is_safe_identifier(&deployment.deployment_id) {
                        bail!(
                            "ssr.deployments[{index}].deployment_id must be 1-128 safe characters"
                        );
                    }
                    if deployment.artifact_path.as_os_str().is_empty() {
                        bail!("ssr.deployments[{index}].artifact_path must not be empty");
                    }
                    if deployment
                        .internal_token
                        .as_deref()
                        .is_some_and(|token| token.trim().len() < 16)
                    {
                        bail!("ssr.deployments[{index}].internal_token must be at least 16 bytes");
                    }
                    let listen = deployment.listen.unwrap_or_else(|| {
                        let offset = u16::try_from(index).unwrap_or(u16::MAX);
                        let port = self.ssr.listen.port().saturating_add(offset);
                        SocketAddr::new(self.ssr.listen.ip(), port)
                    });
                    if listen.port() == 0 || !listen.ip().is_loopback() {
                        bail!(
                            "ssr.deployments[{index}].listen must be loopback with a non-zero port"
                        );
                    }
                    if deployment.preview && deployment.store_id.is_empty() {
                        bail!("preview ssr deployment requires a store_id");
                    }
                    if stores.iter().any(|(store, preview)| {
                        store == &deployment.store_id && *preview == deployment.preview
                    }) {
                        bail!(
                            "duplicate {} ssr deployment for store {}",
                            if deployment.preview {
                                "preview"
                            } else {
                                "production"
                            },
                            deployment.store_id
                        );
                    }
                    if ports.contains(&listen.port()) {
                        bail!("duplicate ssr deployment listen port {}", listen.port());
                    }
                    stores.push((deployment.store_id.clone(), deployment.preview));
                    ports.push(listen.port());
                }
            }
        }
        if self.jobs.enabled && self.jobs.worker_count == 0 {
            bail!("jobs.worker_count must be greater than zero when jobs are enabled");
        }
        if self.database.busy_timeout_ms == 0 {
            bail!("database.busy_timeout_ms must be greater than zero");
        }
        if self.database.max_connections == 0 {
            bail!("database.max_connections must be greater than zero");
        }
        for proxy in &self.server.trusted_proxies {
            if fastcart_http::TrustedProxy::parse(proxy).is_none() {
                bail!(
                    "server.trusted_proxies entries must be IP addresses or CIDRs (found {proxy:?})"
                );
            }
        }
        if let Some(secret) = self.server.preview_secret.as_deref()
            && !secret.trim().is_empty()
            && secret.trim().len() < 32
        {
            bail!("server.preview_secret must be at least 32 bytes when configured");
        }
        for origin in &self.server.preview_frame_ancestors {
            if !valid_origin(origin) {
                bail!(
                    "server.preview_frame_ancestors entries must be bare http(s) origins (found {origin:?})"
                );
            }
        }
        if self.cache.ttl_seconds == 0 {
            bail!("cache.ttl_seconds must be greater than zero");
        }
        if self.inquiries.max_upload_bytes == 0 {
            bail!("inquiries.max_upload_bytes must be greater than zero");
        }
        if self.storage.max_upload_bytes == 0 {
            bail!("storage.max_upload_bytes must be greater than zero");
        }
        if self.inquiries.upload_ttl_seconds <= 0 {
            bail!("inquiries.upload_ttl_seconds must be greater than zero");
        }
        if self.inquiries.retention_days <= 0 {
            bail!("inquiries.retention_days must be greater than zero");
        }
        if self.notifications.retention_days <= 0 {
            bail!("notifications.retention_days must be greater than zero");
        }
        if let Some(url) = self.server.admin_base_url.as_deref() {
            let url = url.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                bail!("server.admin_base_url must be an absolute http(s) URL");
            }
        }
        match self.email.provider.as_str() {
            "log" => {}
            "smtp" => {
                let smtp = self
                    .email
                    .smtp
                    .as_ref()
                    .context("email.smtp is required when email.provider = \"smtp\"")?;
                if smtp.host.trim().is_empty() {
                    bail!("email.smtp.host must not be empty");
                }
                if smtp.port == 0 {
                    bail!("email.smtp.port must be greater than zero");
                }
                if !matches!(smtp.security.as_str(), "implicit" | "starttls" | "plain") {
                    bail!("email.smtp.security must be implicit, starttls or plain");
                }
            }
            "resend" => {
                let resend = self
                    .email
                    .resend
                    .as_ref()
                    .context("email.resend is required when email.provider = \"resend\"")?;
                if resend.api_key.trim().is_empty() {
                    bail!("email.resend.api_key must not be empty");
                }
            }
            other => bail!("email.provider must be log, smtp or resend (found {other})"),
        }
        Ok(())
    }

    /// Returns the configured listener address.
    #[must_use]
    pub fn listen_address(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

/// Returns true for a bare `http(s)://host[:port]` origin.
///
/// The value is placed verbatim in a CSP `frame-ancestors` directive, so
/// paths, queries, whitespace and directive separators are rejected.
fn valid_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && !rest.contains('/')
        && !rest.contains('?')
        && !rest.contains('#')
        && !rest.contains(';')
        && !rest.contains(',')
        && !rest.contains('"')
        && !rest.contains('\'')
        && !rest.chars().any(char::is_whitespace)
}

/// Parses a boolean environment variable value.
fn parse_env_bool(value: &str, variable: &str) -> Result<bool> {
    value
        .trim()
        .parse()
        .with_context(|| format!("{variable} must be true or false"))
}

/// Parses a comma-separated environment variable, trimming entries.
///
/// A wholly empty value clears the list; an empty entry between values is an
/// error because it usually hides a typo.
fn parse_origin_list(value: &str, variable: &str) -> Result<Vec<String>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let origins: Vec<String> = value.split(',').map(str::trim).map(str::to_owned).collect();
    if let Some(index) = origins.iter().position(String::is_empty) {
        bail!("{variable} entry {} must not be empty", index + 1);
    }
    Ok(origins)
}

/// Returns whether a configured identifier is safe for a runtime group and header.
fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Config, SsrDeploymentConfig};

    #[test]
    fn cookie_secure_and_cors_origins_overrides_apply() {
        let mut config = Config::default();
        config
            .apply_server_env(
                Some("false"),
                Some(" https://admin.example.com , https://shop.example.com "),
            )
            .unwrap();
        assert!(!config.server.cookie_secure);
        assert_eq!(
            config.server.cors_origins,
            vec![
                "https://admin.example.com".to_owned(),
                "https://shop.example.com".to_owned()
            ]
        );

        assert!(
            config
                .apply_server_env(Some("yes"), None)
                .unwrap_err()
                .to_string()
                .contains("FASTCART_COOKIE_SECURE")
        );
        assert!(
            config
                .apply_server_env(
                    None,
                    Some("https://admin.example.com,,https://shop.example.com")
                )
                .unwrap_err()
                .to_string()
                .contains("FASTCART_CORS_ORIGINS")
        );

        config
            .apply_server_env(None, Some("   "))
            .expect("an empty value clears the list");
        assert!(config.server.cors_origins.is_empty());
        assert!(config.apply_server_env(None, Some(",")).is_err());
    }

    #[test]
    fn ssr_defaults_are_disabled_and_ssr_invariants_are_enforced() {
        let mut config = Config::default();
        assert!(!config.ssr.enabled);
        config.validate().unwrap();

        config.ssr.enabled = true;
        config.ssr.deployment_id = "bad id".to_owned();
        assert!(config.validate().is_err());

        config.ssr.deployment_id = "default".to_owned();
        config.ssr.listen = "127.0.0.1:0".parse().unwrap();
        assert!(config.validate().is_err());

        config.ssr.listen = "0.0.0.0:18888".parse().unwrap();
        assert!(config.validate().is_err());

        config.ssr.listen = "127.0.0.1:18888".parse().unwrap();
        config.ssr.startup_timeout_ms = 0;
        assert!(config.validate().is_err());

        config.ssr.startup_timeout_ms = 10_000;
        config.ssr.credentials.backend = "vault".to_owned();
        assert!(config.validate().is_err());
        config.ssr.credentials.backend = "file".to_owned();
        config.ssr.credentials.mode = "write".to_owned();
        assert!(config.validate().is_err());
        config.ssr.credentials.mode = "read_only".to_owned();
        config.validate().unwrap();
    }

    #[test]
    fn ssr_deployments_must_be_unique_and_loopback() {
        let mut config = Config::default();
        config.ssr.enabled = true;
        config.ssr.deployments = vec![
            SsrDeploymentConfig {
                store_id: "store_a".to_owned(),
                deployment_id: "deploy_a".to_owned(),
                artifact_path: PathBuf::from("./a"),
                listen: Some("127.0.0.1:18891".parse().unwrap()),
                preview: false,
                internal_token: None,
            },
            SsrDeploymentConfig {
                store_id: "store_b".to_owned(),
                deployment_id: "deploy_b".to_owned(),
                artifact_path: PathBuf::from("./b"),
                listen: Some("127.0.0.1:18892".parse().unwrap()),
                preview: false,
                internal_token: None,
            },
        ];
        config.validate().unwrap();

        config.ssr.deployments[1].store_id = "store_a".to_owned();
        assert!(config.validate().is_err());
        config.ssr.deployments[1].store_id = "store_b".to_owned();

        config.ssr.deployments[1].listen = Some("0.0.0.0:18892".parse().unwrap());
        assert!(config.validate().is_err());
        config.ssr.deployments[1].listen = Some("127.0.0.1:18891".parse().unwrap());
        assert!(config.validate().is_err());

        config.ssr.deployments[1].listen = Some("127.0.0.1:18892".parse().unwrap());
        config.ssr.deployments[1].store_id = "bad id".to_owned();
        assert!(config.validate().is_err());
    }

    #[test]
    fn example_config_parses() {
        let config: Config =
            toml::from_str(include_str!("../../../config/config.example.toml")).unwrap();

        config.validate().unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.ssr.startup_timeout_ms, 10_000);
        assert_eq!(config.ssr.credentials.backend, "file");
        assert_eq!(config.ssr.credentials.mode, "read_write");
        assert_eq!(config.ssr.credentials.encryption, "none");
    }

    #[test]
    fn empty_preview_secret_is_treated_as_unset() {
        let mut config = Config::default();
        config.server.preview_secret = Some("   ".to_owned());
        config.validate().unwrap();

        config.server.preview_secret = Some("too-short".to_owned());
        assert!(config.validate().is_err());
    }

    #[test]
    fn preview_frame_ancestors_must_be_bare_origins() {
        let mut config = Config::default();
        config.server.preview_frame_ancestors = vec!["https://admin.example.com".to_owned()];
        config.validate().unwrap();

        for invalid in [
            "https://admin.example.com/app",
            "admin.example.com",
            "https://admin.example.com,https://other.example.com",
            "https://admin.example.com; script-src *",
            "ftp://admin.example.com",
        ] {
            config.server.preview_frame_ancestors = vec![invalid.to_owned()];
            assert!(config.validate().is_err(), "{invalid:?} must be rejected");
        }
    }
}
