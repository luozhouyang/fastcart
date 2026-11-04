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
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            provider: "local".to_owned(),
            root: PathBuf::from("./data/assets"),
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
    /// Base URL of the Node.js sidecar, used when `runtime = "node"`.
    pub node_url: Option<String>,
    /// Optional bearer secret for sidecar requests.
    pub node_secret: Option<String>,
    /// Optional shared secret enabling the internal storefront HTTP surface.
    pub internal_fetch_secret: Option<String>,
    /// Directory containing trusted theme bundles.
    pub themes_dir: PathBuf,
}

impl Default for StorefrontConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            runtime: "v8".to_owned(),
            worker_count: 2,
            queue_capacity: 128,
            request_timeout_ms: 10_000,
            node_url: None,
            node_secret: None,
            internal_fetch_secret: None,
            themes_dir: PathBuf::from("./themes"),
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
        if let Ok(value) = env::var("FASTCART_PAYMENT_WEBHOOK_SECRET") {
            self.payments.webhook_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STRIPE_SECRET_KEY") {
            self.payments.stripe_secret_key = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STRIPE_WEBHOOK_SECRET") {
            self.payments.stripe_webhook_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STOREFRONT_ENABLED") {
            self.storefront.enabled = value
                .parse()
                .context("FASTCART_STOREFRONT_ENABLED must be true or false")?;
        }
        if let Ok(value) = env::var("FASTCART_STOREFRONT_RUNTIME") {
            self.storefront.runtime = value;
        }
        if let Ok(value) = env::var("FASTCART_STOREFRONT_NODE_URL") {
            self.storefront.node_url = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_STOREFRONT_NODE_SECRET") {
            self.storefront.node_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_INTERNAL_FETCH_SECRET") {
            self.storefront.internal_fetch_secret = Some(value);
        }
        if let Ok(value) = env::var("FASTCART_THEMES_DIR") {
            self.storefront.themes_dir = PathBuf::from(value);
        }
        if let Ok(value) = env::var("FASTCART_PREVIEW_SECRET") {
            self.server.preview_secret = Some(value);
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
        if !matches!(self.storefront.runtime.as_str(), "v8" | "node") {
            bail!("storefront.runtime must be either \"v8\" or \"node\"");
        }
        if self.storefront.enabled
            && self.storefront.runtime == "node"
            && self
                .storefront
                .node_url
                .as_deref()
                .is_none_or(str::is_empty)
        {
            bail!("storefront.node_url is required when the node runtime is enabled");
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
        if let Some(secret) = self.server.preview_secret.as_deref()
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
