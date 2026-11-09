//! API process state shared by all routes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use axum::response::IntoResponse;
use tokio::sync::Mutex;

use fastcart_email::EmailService;
use fastcart_http::{Metrics, RateLimiter};
use fastcart_infrastructure::{EventBus, LocalStorage, Storage, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{StoreContext, StoreId};
use fastcart_module_payment::{ManualProvider, ProviderRegistry, StripeProvider};
use fastcart_platform::{AuthService, PlatformStore, StoreProvisioningService, StoreResolver};
use fastcart_runtime_api::StorefrontRuntime;

/// Process-wide universal SSR runtime.
#[async_trait::async_trait]
pub trait UniversalStorefront: Send + Sync {
    /// Forwards a storefront request to the configured artifact runtime.
    async fn render(
        &self,
        request: axum::http::Request<axum::body::Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> axum::response::Response;

    /// Serves one client asset from the active artifact.
    async fn serve_asset(
        &self,
        store_id: &str,
        path: String,
        headers: axum::http::HeaderMap,
    ) -> axum::response::Response;

    /// Starts and activates a new deployment for one store.
    async fn activate(
        &self,
        store_id: &str,
        deployment_id: &str,
        artifact_path: String,
    ) -> Result<(), String>;

    /// Restores the previous deployment for one store.
    async fn rollback(&self, store_id: &str) -> Result<(), String>;

    /// Invalidates cached SSR responses for one store.
    async fn invalidate_cache(&self, _store_id: &str) {}

    /// Renders through the store's preview deployment.
    async fn render_preview(
        &self,
        _request: axum::http::Request<axum::body::Body>,
        _store: StoreContext,
        _request_id: Option<String>,
        _scheme: String,
    ) -> axum::response::Response {
        axum::http::StatusCode::NOT_FOUND.into_response()
    }

    /// Returns whether the store has a preview deployment.
    async fn has_preview(&self, _store_id: &str) -> bool {
        false
    }

    /// Starts and activates a preview deployment.
    async fn activate_preview(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Err("preview deployments are not supported".to_owned())
    }

    /// Stops the store's preview deployment.
    async fn deactivate_preview(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }
}

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
    /// Timeout applied to storefront SSR render requests in milliseconds.
    pub storefront_timeout_ms: u64,
    /// Whether runtimes may stream storefront responses.
    ///
    /// Enabled by default. Disabling it makes every render buffered, which is
    /// useful behind proxies that cannot pass chunked responses through.
    pub storefront_streaming_enabled: bool,
    /// Total time budget for one streamed storefront response in milliseconds.
    ///
    /// A stream still has to produce its shell within
    /// [`Self::storefront_timeout_ms`]; this larger budget bounds the whole
    /// body.
    pub storefront_stream_timeout_ms: u64,
    /// Optional shared secret required by the internal storefront HTTP surface.
    ///
    /// When absent, internal routes are not mounted over HTTP; the in-process
    /// bridge remains the only way to reach them.
    pub internal_fetch_secret: Option<String>,
    /// Optional bearer token for the server-to-server SSR commerce API.
    ///
    /// When absent, `/internal/ssr/v1` is not mounted.
    pub internal_ssr_token: Option<String>,
    /// Optional secret used to sign theme capability grants.
    ///
    /// When set, grants carry a signed payload a capability endpoint sharing
    /// the secret can verify statelessly. Unset mints opaque random tokens.
    pub capability_secret: Option<String>,
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
    /// Root directory containing trusted theme bundles.
    ///
    /// Used to load `editor.manifest.json` at registration and to keep bundle
    /// paths confined to deployed theme code.
    pub themes_root: PathBuf,
    /// Runtime provider selected for this deployment (`v8` or `node`).
    ///
    /// Theme activation compares this with the manifest's declared runtime so a
    /// theme cannot be activated onto an incompatible runtime.
    pub storefront_runtime_kind: String,
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
            storefront_timeout_ms: 10_000,
            storefront_streaming_enabled: true,
            storefront_stream_timeout_ms: 30_000,
            internal_fetch_secret: None,
            internal_ssr_token: None,
            capability_secret: None,
            preview_secret: None,
            preview_frame_ancestors: Vec::new(),
            themes_root: PathBuf::from("./themes"),
            storefront_runtime_kind: "node".to_owned(),
            admin_base_url: None,
            storage_root: PathBuf::from("./data/assets"),
            max_asset_upload_bytes: 100 * 1024 * 1024,
            max_upload_bytes: 25 * 1024 * 1024,
            upload_ttl_seconds: 3_600,
            inquiry_retention_days: 365,
            notification_retention_days: 90,
        }
    }
}

/// Maximum number of parsed theme editor manifests retained per process.
const STOREFRONT_EDITOR_CACHE_CAPACITY: u64 = 1024;
/// Maximum weighted bytes retained by the compiled page cache.
///
/// The cache weighs each entry by its serialized page body plus bindings, so
/// many large pages cannot exhaust memory even though each body is capped at
/// 1 MiB.
const STOREFRONT_PAGE_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// Lifetime of compiled storefront entries.
const STOREFRONT_CACHE_TTL: Duration = Duration::from_secs(300);
/// Lifetime of the request-context customer and cart lookups.
///
/// These cache entries carry live commerce data, so the window is deliberately
/// short: a mutation that bypasses explicit invalidation becomes visible within
/// a couple of seconds, while bursts of SSR requests for the same visitor share
/// one database round trip.
const STOREFRONT_CONTEXT_CACHE_TTL: Duration = Duration::from_secs(2);
/// Maximum number of cached customer sessions and cart snapshots per process.
const STOREFRONT_CONTEXT_CACHE_CAPACITY: u64 = 10_000;

/// A validated page and the data bindings extracted in the same pass.
///
/// The compiled form is cached per store, page revision and active theme so a
/// render neither re-parses nor re-validates an unchanged page. Cached values
/// never carry live commerce data.
#[derive(Clone, Debug)]
pub struct CompiledPage {
    /// Page metadata and validated body handed to the runtime.
    pub page: fastcart_runtime_api::PageContext,
    /// Bindings extracted from the validated body.
    pub bindings: Vec<fastcart_storefront::BindingRequest>,
}

impl CompiledPage {
    /// Estimates the memory retained by this cached entry.
    ///
    /// The estimate is intentionally cheap and slightly conservative: the
    /// serialized body dominates, while bindings are small fixed-shape values.
    #[must_use]
    pub fn cache_weight(&self) -> u32 {
        let body = serde_json::to_string(&self.page.body)
            .map(|body| body.len())
            .unwrap_or(0);
        let bindings = self.bindings.len().saturating_mul(256);
        u32::try_from(body.saturating_add(bindings)).unwrap_or(u32::MAX)
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
    /// Optional storefront SSR runtime shared by all stores.
    pub storefront_runtime: Option<Arc<dyn StorefrontRuntime>>,
    /// Optional universal workerd storefront runtime.
    pub universal_storefront: Option<Arc<dyn UniversalStorefront>>,
    /// Server-side resolver for theme data bindings.
    pub storefront_data: fastcart_storefront_data::StorefrontDataResolver,
    /// Parsed active theme editor manifests keyed by store, theme and revision.
    ///
    /// The value is `None` when the stored manifest is absent or invalid, so
    /// invalid manifests are not re-parsed on every render.
    pub storefront_editor_cache:
        moka::future::Cache<String, Option<Arc<fastcart_storefront::ThemeEditorManifest>>>,
    /// Compiled storefront pages keyed by store, page revision and theme.
    pub storefront_page_cache: moka::future::Cache<String, Arc<CompiledPage>>,
    /// Recently resolved storefront customer sessions.
    ///
    /// Keyed by store and a digest of the session token so raw tokens are not
    /// retained in the cache. Values are `None` for invalid sessions.
    pub storefront_customer_cache:
        moka::future::Cache<String, Option<Arc<fastcart_module_customer::Customer>>>,
    /// Recently resolved storefront cart snapshots for the request context.
    ///
    /// Keyed by store plus customer id (authenticated) or cart id (guest).
    /// Explicitly invalidated by cart mutations; the short TTL bounds staleness
    /// for writes that do not pass through those handlers.
    pub storefront_cart_cache: moka::future::Cache<String, Option<Arc<fastcart_module_cart::Cart>>>,
    /// Provider-independent transactional email delivery.
    pub email: EmailService,
    /// Private object storage used for inquiry attachments.
    pub storage: Arc<dyn Storage>,
    /// Public inquiry submission limiter, keyed by store and client address.
    pub inquiry_limiter: RateLimiter,
    /// Attachment upload limiter, keyed by store and client address.
    pub upload_limiter: RateLimiter,
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
            webhook_limiter: RateLimiter::new(600, Duration::from_secs(60)),
            login_limiter: RateLimiter::new(10, Duration::from_secs(5 * 60)),
            customer_email_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
            bootstrap_lock: Arc::new(Mutex::new(())),
            payment_providers,
            payment_providers_cache: Arc::new(StdMutex::new(HashMap::new())),
            shipping_providers:
                fastcart_module_shipping::FulfillmentProviderRegistry::with_defaults(),
            metrics: Arc::new(Metrics::new()),
            storefront_runtime: None,
            universal_storefront: None,
            storefront_data: fastcart_storefront_data::StorefrontDataResolver::new(),
            storefront_editor_cache: moka::future::Cache::builder()
                .max_capacity(STOREFRONT_EDITOR_CACHE_CAPACITY)
                .time_to_live(STOREFRONT_CACHE_TTL)
                .build(),
            storefront_page_cache: moka::future::Cache::builder()
                .max_capacity(STOREFRONT_PAGE_CACHE_BYTES)
                .weigher(|_key: &String, value: &Arc<CompiledPage>| value.cache_weight())
                .time_to_live(STOREFRONT_CACHE_TTL)
                .build(),
            storefront_customer_cache: moka::future::Cache::builder()
                .max_capacity(STOREFRONT_CONTEXT_CACHE_CAPACITY)
                .time_to_live(STOREFRONT_CONTEXT_CACHE_TTL)
                .build(),
            storefront_cart_cache: moka::future::Cache::builder()
                .max_capacity(STOREFRONT_CONTEXT_CACHE_CAPACITY)
                .time_to_live(STOREFRONT_CONTEXT_CACHE_TTL)
                .build(),
            email: EmailService::with_default_registry(),
            storage,
            inquiry_limiter: RateLimiter::new(5, Duration::from_secs(10 * 60)),
            upload_limiter: RateLimiter::new(10, Duration::from_secs(60 * 60)),
        }
    }

    /// Installs the process-wide storefront runtime, if SSR is configured.
    #[must_use]
    pub fn with_storefront_runtime(mut self, runtime: Option<Arc<dyn StorefrontRuntime>>) -> Self {
        self.storefront_runtime = runtime;
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

    /// Drops cached storefront cart snapshots after a cart mutation.
    pub fn invalidate_storefront_carts(&self) {
        self.storefront_cart_cache.invalidate_all();
    }

    /// Drops cached storefront customer sessions after login, logout or revocation.
    pub fn invalidate_storefront_customers(&self) {
        self.storefront_customer_cache.invalidate_all();
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
