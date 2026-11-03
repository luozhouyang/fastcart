//! API process state shared by all routes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fastcart_http::{Metrics, RateLimiter};
use fastcart_infrastructure::{EventBus, StoreCache, StoreDatabaseManager};
use fastcart_module_payment::{ManualProvider, ProviderRegistry};
use fastcart_platform::{AuthService, PlatformStore, StoreProvisioningService, StoreResolver};

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
    /// Registered payment providers shared by all stores.
    pub payment_providers: ProviderRegistry,
    /// HTTP request metrics.
    pub metrics: Arc<Metrics>,
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
            payment_providers,
            metrics: Arc::new(Metrics::new()),
        }
    }
}
