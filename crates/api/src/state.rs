//! API process state shared by all routes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use fastcart_email::EmailService;
use fastcart_http::{Metrics, RateLimiter};
use fastcart_infrastructure::{EventBus, StoreCache, StoreDatabaseManager};
use fastcart_module_payment::{ManualProvider, ProviderRegistry};
use fastcart_platform::{AuthService, PlatformStore, StoreProvisioningService, StoreResolver};
use fastcart_runtime_api::StorefrontRuntime;

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
    /// Maximum accepted request body size in bytes.
    pub max_body_bytes: usize,
    /// Directory used for store backups requested through the API.
    pub backup_dir: PathBuf,
    /// Shared secret used to verify sandbox/manual payment webhooks.
    pub manual_payment_secret: Option<String>,
    /// Timeout applied to storefront SSR render requests in milliseconds.
    pub storefront_timeout_ms: u64,
    /// Optional shared secret required by the internal storefront HTTP surface.
    ///
    /// When absent, internal routes are not mounted over HTTP; the in-process
    /// bridge remains the only way to reach them.
    pub internal_fetch_secret: Option<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            service_name: "fastcart".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            cookie_secure: true,
            allowed_origins: Vec::new(),
            max_body_bytes: 2 * 1024 * 1024,
            backup_dir: PathBuf::from("./data/backups"),
            manual_payment_secret: None,
            storefront_timeout_ms: 10_000,
            internal_fetch_secret: None,
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
    /// Login attempt limiter.
    pub login_limiter: RateLimiter,
    /// Serializes first-user bootstrap so concurrent setup calls cannot both win.
    pub bootstrap_lock: Arc<Mutex<()>>,
    /// Registered payment providers shared by all stores.
    pub payment_providers: ProviderRegistry,
    /// Registered fulfillment providers shared by all stores.
    pub shipping_providers: fastcart_module_shipping::FulfillmentProviderRegistry,
    /// HTTP request metrics.
    pub metrics: Arc<Metrics>,
    /// Optional storefront SSR runtime shared by all stores.
    pub storefront_runtime: Option<Arc<dyn StorefrontRuntime>>,
    /// Provider-independent transactional email delivery.
    pub email: EmailService,
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
        let auth = AuthService::new(platform.clone());
        let resolver = StoreResolver::new(platform.clone(), cache.clone());
        let provisioning = StoreProvisioningService::new(platform.clone(), databases.clone());
        let payment_providers = ProviderRegistry::new();
        let provider = ManualProvider::new("manual", config.manual_payment_secret.clone());
        if let Err(error) = payment_providers.register(std::sync::Arc::new(provider)) {
            tracing::warn!(error = %error, "failed to register the manual payment provider");
        }
        Self {
            config,
            platform,
            auth,
            provisioning,
            resolver,
            databases,
            events: EventBus::new(1024),
            cache,
            login_limiter: RateLimiter::new(10, Duration::from_secs(5 * 60)),
            bootstrap_lock: Arc::new(Mutex::new(())),
            payment_providers,
            shipping_providers:
                fastcart_module_shipping::FulfillmentProviderRegistry::with_defaults(),
            metrics: Arc::new(Metrics::new()),
            storefront_runtime: None,
            email: EmailService::with_default_registry(),
        }
    }

    /// Installs the process-wide storefront runtime, if SSR is configured.
    #[must_use]
    pub fn with_storefront_runtime(mut self, runtime: Option<Arc<dyn StorefrontRuntime>>) -> Self {
        self.storefront_runtime = runtime;
        self
    }

    /// Replaces the email service with one configured for this deployment.
    #[must_use]
    pub fn with_email(mut self, email: EmailService) -> Self {
        self.email = email;
        self
    }
}
