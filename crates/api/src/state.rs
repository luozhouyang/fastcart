//! API process state shared by all routes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::Mutex;

use fastcart_email::EmailService;
use fastcart_http::{Metrics, RateLimiter};
use fastcart_infrastructure::{EventBus, LocalStorage, Storage, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_module_payment::{ManualProvider, ProviderRegistry, StripeProvider};
use fastcart_platform::{AuthService, PlatformStore, StoreProvisioningService, StoreResolver};
use fastcart_plugin::{
    ArchiveLimits, ArtifactStore, EgressLimiter, PluginHost, RuntimeConfig, SecretCipher,
    TokenSigner,
};

pub use fastcart_ssr::UniversalStorefront;

pub use fastcart_ssr_credentials::{
    CredentialManager as InternalSsrCredentials, CredentialMode,
    CredentialToken as InternalSsrToken, FileCredentialBackend,
};

/// HTTP-facing configuration.
#[derive(Clone, Debug)]
pub struct ApiConfig {
    /// Service name reported by health endpoints.
    pub service_name: String,
    /// Service version reported by health endpoints.
    pub version: String,
    /// Whether session cookies carry the Secure attribute.
    pub cookie_secure: bool,
    /// Exact browser origins allowed to call the API with credentials.
    pub allowed_origins: Vec<String>,
    /// Exact addresses or CIDRs whose forwarded headers are trusted.
    pub trusted_proxies: Vec<String>,
    /// Maximum accepted request body size in bytes.
    pub max_body_bytes: usize,
    /// Directory used for store backups requested through the API.
    pub backup_dir: PathBuf,
    /// Shared secret used to verify sandbox/manual payment webhooks.
    pub manual_payment_secret: Option<String>,
    /// Stripe secret API key.
    ///
    /// The Stripe provider is only registered when this and
    /// [`Self::stripe_webhook_secret`] are both configured.
    pub stripe_secret_key: Option<String>,
    /// Stripe webhook signing secret.
    pub stripe_webhook_secret: Option<String>,
    /// Optional bearer token for the server-to-server SSR commerce API.
    ///
    /// When absent, `/internal/ssr/v1` is not mounted.
    pub internal_ssr_token: Option<String>,
    /// Mutable scoped SSR credentials.
    ///
    /// When empty, [`Self::internal_ssr_token`] is used as an unscoped
    /// credential.
    pub internal_ssr_credentials: InternalSsrCredentials,
    /// Optional signing secret for draft page preview tokens.
    ///
    /// When absent, preview token issuance and the preview render surface are
    /// disabled and respond with `501 Not Implemented`. The secret must be
    /// supplied out of band and must never be logged.
    pub preview_secret: Option<String>,
    /// Origins permitted to embed the preview surface in an iframe.
    ///
    /// Empty leaves framing unrestricted (the token remains the credential).
    /// Deployments should list the admin application origin.
    pub preview_frame_ancestors: Vec<String>,
    /// Optional admin application origin used for notification deep links.
    pub admin_base_url: Option<String>,
    /// Root directory backing the default local object storage adapter.
    pub storage_root: PathBuf,
    /// Hard cap for one media-library asset upload.
    pub max_asset_upload_bytes: usize,
    /// Hard cap for one inquiry attachment upload.
    pub max_upload_bytes: usize,
    /// Lifetime of a pending inquiry attachment upload.
    pub upload_ttl_seconds: i64,
    /// Days an inquiry and its attachments are retained.
    pub inquiry_retention_days: i64,
    /// Days terminal notification channel deliveries are retained.
    pub notification_retention_days: i64,
    /// Root directory for content-addressed plugin artifacts.
    pub plugins_root: PathBuf,
    /// Hard cap for one plugin archive upload.
    pub max_plugin_archive_bytes: usize,
    /// Hard cap for one plugin JavaScript module.
    pub max_plugin_module_bytes: u64,
    /// Whether plugin installations and enabling are accepted.
    pub plugins_enabled: bool,
    /// Optional bearer token for the internal plugin runtime endpoint.
    ///
    /// When absent, `/internal/plugins/v1` is not mounted.
    pub plugin_runtime_token: Option<String>,
    /// Pinned workerd binary used by the plugin host.
    pub plugin_workerd_binary: PathBuf,
    /// Directory for generated runtime generations. Empty derives from the
    /// artifact root.
    pub plugin_runtime_dir: PathBuf,
    /// Address of this server as seen from workerd.
    pub plugin_commerce_addr: SocketAddr,
    /// Secret used to sign plugin context tokens. Random per process when
    /// absent.
    pub plugin_context_secret: Option<Vec<u8>>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            service_name: "fastcart".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            cookie_secure: true,
            allowed_origins: Vec::new(),
            trusted_proxies: Vec::new(),
            max_body_bytes: 2 * 1024 * 1024,
            backup_dir: PathBuf::from("./data/backups"),
            manual_payment_secret: None,
            stripe_secret_key: None,
            stripe_webhook_secret: None,
            internal_ssr_token: None,
            internal_ssr_credentials: InternalSsrCredentials::default(),
            preview_secret: None,
            preview_frame_ancestors: Vec::new(),
            admin_base_url: None,
            storage_root: PathBuf::from("./data/assets"),
            max_asset_upload_bytes: 100 * 1024 * 1024,
            max_upload_bytes: 25 * 1024 * 1024,
            upload_ttl_seconds: 3_600,
            inquiry_retention_days: 365,
            notification_retention_days: 90,
            plugins_root: PathBuf::from("./data/plugins"),
            max_plugin_archive_bytes: 20 * 1024 * 1024,
            max_plugin_module_bytes: 4 * 1024 * 1024,
            plugins_enabled: false,
            plugin_runtime_token: None,
            plugin_workerd_binary: PathBuf::from("experiments/workerd/bin/workerd"),
            plugin_runtime_dir: PathBuf::new(),
            plugin_commerce_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
            plugin_context_secret: None,
        }
    }
}

/// Process-wide services used by the HTTP layer.
pub struct AppContext {
    /// HTTP configuration.
    pub config: ApiConfig,
    /// Platform database.
    pub platform: PlatformStore,
    /// Authentication and RBAC.
    pub auth: AuthService,
    /// Store provisioning and lifecycle.
    pub provisioning: StoreProvisioningService,
    /// Trusted store resolution.
    pub resolver: StoreResolver,
    /// Per-store database manager.
    pub databases: Arc<StoreDatabaseManager>,
    /// In-process event bus.
    pub events: EventBus,
    /// Store-aware cache.
    pub cache: StoreCache,
    /// Resolved trusted client-IP configuration.
    pub client_ip: fastcart_http::ClientIpConfig,
    /// Cart creation limiter, keyed by client address.
    pub cart_create_limiter: RateLimiter,
    /// Cart line mutation limiter, keyed by client address.
    pub cart_write_limiter: RateLimiter,
    /// Checkout creation/completion limiter, keyed by client address.
    pub checkout_limiter: RateLimiter,
    /// Draft preview render limiter, keyed by client address.
    pub preview_limiter: RateLimiter,
    /// Short-lived positive cache for preview asset revocation checks.
    pub preview_asset_authorizations: moka::future::Cache<String, ()>,
    /// Payment webhook limiter, keyed by client address.
    pub webhook_limiter: RateLimiter,
    /// Login attempt limiter.
    pub login_limiter: RateLimiter,
    /// Customer email-verification and password-reset request limiter.
    pub customer_email_limiter: RateLimiter,
    /// Serializes first-user bootstrap so concurrent setup calls cannot both win.
    pub bootstrap_lock: Arc<Mutex<()>>,
    /// Registered payment providers shared by all stores.
    pub payment_providers: ProviderRegistry,
    /// Per-store provider registries, built lazily from env overrides.
    payment_providers_cache: Arc<StdMutex<HashMap<String, ProviderRegistry>>>,
    /// Registered fulfillment providers shared by all stores.
    pub shipping_providers: fastcart_module_shipping::FulfillmentProviderRegistry,
    /// HTTP request metrics.
    pub metrics: Arc<Metrics>,
    /// Optional universal workerd storefront runtime.
    pub universal_storefront: Option<Arc<dyn UniversalStorefront>>,
    /// Provider-independent transactional email delivery.
    pub email: EmailService,
    /// Private object storage used for inquiry attachments.
    pub storage: Arc<dyn Storage>,
    /// Public inquiry submission limiter, keyed by store and client address.
    pub inquiry_limiter: RateLimiter,
    /// Attachment upload limiter, keyed by store and client address.
    pub upload_limiter: RateLimiter,
    /// Content-addressed plugin artifact store shared by all stores.
    pub plugin_artifacts: Arc<ArtifactStore>,
    /// Bearer token shared by the gateway and the bundle endpoint.
    pub plugin_runtime_token: Option<String>,
    /// Lazy workerd host, absent when plugins are disabled or misconfigured.
    pub plugin_host: Option<Arc<PluginHost>>,
    /// Signer for short-lived plugin context tokens.
    pub plugin_tokens: TokenSigner,
    /// Encrypts per-plugin secrets at rest.
    pub plugin_secret_cipher: Arc<SecretCipher>,
    /// Whether the secret key came from deployment configuration.
    ///
    /// A generated key changes on every restart, so secret writes are refused
    /// unless the operator configured `[plugins] context_secret`.
    pub plugin_secrets_durable: bool,
    /// Allowlisted egress executor with daily quotas.
    pub plugin_egress: Arc<EgressLimiter>,
    /// Single-use `(jti, call id)` pairs for bridge calls.
    pub plugin_call_replay: moka::future::Cache<String, ()>,
    /// Public `/apps/{plugin}` traffic limiter, keyed by store, plugin and
    /// client address.
    pub plugin_apps_limiter: RateLimiter,
}

impl AppContext {
    /// Returns the payment providers for one store.
    ///
    /// Global providers are reused when a store has no env overrides. Stripe
    /// credentials can be scoped per store with:
    ///
    /// - `FASTCART_STORE_<NORMALIZED_STORE_ID>_STRIPE_SECRET_KEY`
    /// - `FASTCART_STORE_<NORMALIZED_STORE_ID>_STRIPE_WEBHOOK_SECRET`
    /// - `FASTCART_STORE_<NORMALIZED_STORE_ID>_MANUAL_PAYMENT_SECRET`
    ///
    /// The store ID is upper-cased and every non-alphanumeric byte becomes `_`.
    /// Global `FASTCART_STRIPE_*` values remain the fallback for stores without
    /// overrides. Registries are cached per store because env is immutable for
    /// the lifetime of the process.
    #[must_use]
    pub fn payment_providers_for(&self, store_id: &StoreId) -> ProviderRegistry {
        let key = store_id.to_string();
        if let Some(registry) = self
            .payment_providers_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
        {
            return registry.clone();
        }

        let store_secret = store_provider_env(store_id, "STRIPE_SECRET_KEY");
        let store_webhook = store_provider_env(store_id, "STRIPE_WEBHOOK_SECRET");
        let store_manual = store_provider_env(store_id, "MANUAL_PAYMENT_SECRET");
        let registry = if store_secret.is_none()
            && store_webhook.is_none()
            && store_manual.is_none()
        {
            self.payment_providers.clone()
        } else {
            let registry = ProviderRegistry::new();
            let manual = ManualProvider::new(
                "manual",
                store_manual.or_else(|| self.config.manual_payment_secret.clone()),
            );
            if let Err(error) = registry.register(std::sync::Arc::new(manual)) {
                tracing::warn!(error = %error, "failed to register the manual payment provider");
            }
            register_stripe_provider_with(
                &registry,
                store_secret
                    .as_deref()
                    .or(self.config.stripe_secret_key.as_deref()),
                store_webhook
                    .as_deref()
                    .or(self.config.stripe_webhook_secret.as_deref()),
            );
            registry
        };

        self.payment_providers_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, registry.clone());
        registry
    }
}

/// Reads a per-store provider override from the environment.
fn store_provider_env(store_id: &StoreId, name: &str) -> Option<String> {
    let normalized = store_env_suffix(store_id);
    std::env::var(format!("FASTCART_STORE_{normalized}_{name}"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Upper-cases a store ID and replaces every non-alphanumeric byte with `_`.
fn store_env_suffix(store_id: &StoreId) -> String {
    store_id
        .as_str()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl std::fmt::Debug for AppContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppContext")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AppContext {
    /// Builds the API context from infrastructure services.
    #[must_use]
    pub fn new(
        config: ApiConfig,
        platform: PlatformStore,
        databases: Arc<StoreDatabaseManager>,
        cache: StoreCache,
    ) -> Self {
        let mut config = config;
        if config.internal_ssr_credentials.is_empty()
            && let Some(token) = config
                .internal_ssr_token
                .clone()
                .filter(|token| !token.is_empty())
        {
            config.internal_ssr_credentials = InternalSsrCredentials::new(vec![InternalSsrToken {
                token,
                store_id: None,
                expires_at: None,
                preview: false,
            }]);
        }
        let auth = AuthService::new(platform.clone());
        let resolver = StoreResolver::new(platform.clone(), cache.clone());
        let provisioning = StoreProvisioningService::new(platform.clone(), databases.clone());
        let payment_providers = ProviderRegistry::new();
        let provider = ManualProvider::new("manual", config.manual_payment_secret.clone());
        if let Err(error) = payment_providers.register(std::sync::Arc::new(provider)) {
            tracing::warn!(error = %error, "failed to register the manual payment provider");
        }
        register_stripe_provider(&payment_providers, &config);
        let storage: Arc<dyn Storage> = Arc::new(LocalStorage::new(config.storage_root.clone()));
        let client_ip = fastcart_http::ClientIpConfig::parse(&config.trusted_proxies);
        let plugin_secrets_durable = config.plugin_context_secret.is_some();
        let plugin_context_secret = config
            .plugin_context_secret
            .clone()
            .unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string().into_bytes());
        let plugin_tokens = TokenSigner::new(&plugin_context_secret);
        let plugin_secret_cipher = Arc::new(SecretCipher::new(&plugin_context_secret));
        let plugin_runtime_token = config
            .plugin_runtime_token
            .clone()
            .filter(|token| !token.is_empty())
            .or_else(|| {
                config.plugins_enabled.then(|| {
                    format!(
                        "fc_rt_{}{}",
                        uuid::Uuid::now_v7().simple(),
                        uuid::Uuid::now_v7().simple()
                    )
                })
            });
        let plugin_host = if config.plugins_enabled {
            let runtime_dir = if config.plugin_runtime_dir.as_os_str().is_empty() {
                config.plugins_root.join("runtime")
            } else {
                config.plugin_runtime_dir.clone()
            };
            let runtime = RuntimeConfig {
                binary: config.plugin_workerd_binary.clone(),
                runtime_dir,
                commerce_addr: config.plugin_commerce_addr,
                runtime_token: plugin_runtime_token.clone().unwrap_or_default(),
                ..RuntimeConfig::default()
            };
            match PluginHost::new(runtime) {
                Ok(host) => Some(Arc::new(host)),
                Err(error) => {
                    tracing::warn!(error = %error, "plugin host is unavailable");
                    None
                }
            }
        } else {
            None
        };
        let plugin_artifacts = Arc::new(ArtifactStore::new(
            config.plugins_root.clone(),
            ArchiveLimits {
                max_archive_bytes: config.max_plugin_archive_bytes as u64,
                ..ArchiveLimits::default()
            },
            config.max_plugin_module_bytes,
        ));
        Self {
            config,
            platform,
            auth,
            provisioning,
            resolver,
            databases,
            events: EventBus::new(1024),
            cache,
            client_ip,
            cart_create_limiter: RateLimiter::new(30, Duration::from_secs(60)),
            cart_write_limiter: RateLimiter::new(120, Duration::from_secs(60)),
            checkout_limiter: RateLimiter::new(20, Duration::from_secs(60)),
            preview_limiter: RateLimiter::new(120, Duration::from_secs(60)),
            preview_asset_authorizations: moka::future::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(30))
                .build(),
            webhook_limiter: RateLimiter::new(600, Duration::from_secs(60)),
            login_limiter: RateLimiter::new(10, Duration::from_secs(5 * 60)),
            customer_email_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
            bootstrap_lock: Arc::new(Mutex::new(())),
            payment_providers,
            payment_providers_cache: Arc::new(StdMutex::new(HashMap::new())),
            shipping_providers:
                fastcart_module_shipping::FulfillmentProviderRegistry::with_defaults(),
            metrics: Arc::new(Metrics::new()),
            universal_storefront: None,
            email: EmailService::with_default_registry(),
            storage,
            inquiry_limiter: RateLimiter::new(5, Duration::from_secs(10 * 60)),
            upload_limiter: RateLimiter::new(10, Duration::from_secs(60 * 60)),
            plugin_artifacts,
            plugin_runtime_token,
            plugin_host,
            plugin_tokens,
            plugin_secret_cipher,
            plugin_secrets_durable,
            plugin_egress: Arc::new(EgressLimiter::new()),
            plugin_call_replay: moka::future::Cache::builder()
                .max_capacity(100_000)
                .time_to_live(Duration::from_secs(60))
                .build(),
            plugin_apps_limiter: RateLimiter::new(120, Duration::from_secs(60)),
        }
    }

    /// Replaces the process metrics collector.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// Installs the universal workerd storefront runtime.
    #[must_use]
    pub fn with_universal_storefront(
        mut self,
        runtime: Option<Arc<dyn UniversalStorefront>>,
    ) -> Self {
        self.universal_storefront = runtime;
        self
    }

    /// Replaces the email service with one configured for this deployment.
    #[must_use]
    pub fn with_email(mut self, email: EmailService) -> Self {
        self.email = email;
        self
    }

    /// Replaces the attachment storage adapter.
    #[must_use]
    pub fn with_storage(mut self, storage: Arc<dyn Storage>) -> Self {
        self.storage = storage;
        self
    }
}

/// Registers the Stripe provider when both credentials are present.
fn register_stripe_provider(registry: &ProviderRegistry, config: &ApiConfig) {
    register_stripe_provider_with(
        registry,
        config.stripe_secret_key.as_deref(),
        config.stripe_webhook_secret.as_deref(),
    );
}

fn register_stripe_provider_with(
    registry: &ProviderRegistry,
    secret_key: Option<&str>,
    webhook_secret: Option<&str>,
) {
    let secret_key = secret_key.map(str::trim).filter(|value| !value.is_empty());
    let webhook_secret = webhook_secret
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (secret_key, webhook_secret) {
        (Some(secret_key), Some(webhook_secret)) => {
            let provider = StripeProvider::new(secret_key, Some(webhook_secret.to_owned()));
            if let Err(error) = registry.register(std::sync::Arc::new(provider)) {
                tracing::warn!(error = %error, "failed to register the Stripe payment provider");
            }
        }
        (Some(_), None) => {
            tracing::warn!(
                "FASTCART_STRIPE_WEBHOOK_SECRET is not configured; the Stripe payment provider is disabled"
            );
        }
        (None, Some(_)) => {
            tracing::warn!(
                "FASTCART_STRIPE_SECRET_KEY is not configured; the Stripe payment provider is disabled"
            );
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod provider_tests {
    use fastcart_kernel::StoreId;

    use super::store_env_suffix;

    #[test]
    fn store_env_suffix_is_uppercase_and_safe() {
        let store = StoreId::from_string("store_01ab-cd.ef");
        assert_eq!(store_env_suffix(&store), "STORE_01AB_CD_EF");
        let plain = StoreId::from_string("abc123");
        assert_eq!(store_env_suffix(&plain), "ABC123");
    }
}
