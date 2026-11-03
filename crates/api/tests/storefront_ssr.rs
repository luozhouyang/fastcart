//! Storefront SSR fallback and internal bridge security.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_module_theme::{RegisterThemeInput, ThemeService};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_runtime_api::{RuntimeError, RuntimeRequest, RuntimeResponse, StorefrontRuntime};
use fastcart_storefront::ThemeManifest;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[derive(Clone)]
struct RecordingRuntime {
    requests: Arc<Mutex<Vec<RuntimeRequest>>>,
}

impl RecordingRuntime {
    fn new() -> (Self, Arc<Mutex<Vec<RuntimeRequest>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                requests: requests.clone(),
            },
            requests,
        )
    }
}

#[async_trait::async_trait]
impl StorefrontRuntime for RecordingRuntime {
    async fn render(&self, request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        self.requests.lock().unwrap().push(request);
        Ok(RuntimeResponse {
            status: 200,
            headers: vec![("content-type".to_owned(), "text/html".to_owned())],
            body: fastcart_runtime_api::RuntimeBody::full("<h1>rendered</h1>"),
        })
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

struct TimeoutRuntime;

#[async_trait::async_trait]
impl StorefrontRuntime for TimeoutRuntime {
    async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        Err(RuntimeError::Timeout(25))
    }

    async fn health_check(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
}

struct Fixture {
    app: Router,
    _dir: tempfile::TempDir,
}

async fn fixture(
    runtime: Option<Arc<dyn StorefrontRuntime>>,
    internal_secret: Option<String>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let store = platform
        .create_store(CreateStoreInput::new("Demo", "demo", "shop.example.com"))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();

    let database = databases.database(&store.id).await.unwrap();
    let themes = ThemeService::new(database);
    let theme = themes
        .register(RegisterThemeInput {
            manifest: ThemeManifest {
                id: "default".to_owned(),
                name: "Default".to_owned(),
                version: "1.0.0".to_owned(),
                runtime: "node".to_owned(),
                entry: "dist/server.js".to_owned(),
                settings_schema: None,
                assets_manifest: None,
            },
            bundle_path: "themes/default".to_owned(),
            settings_schema: None,
            settings: Some(serde_json::json!({"brand_color": "#111111"})),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                internal_fetch_secret: internal_secret,
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_storefront_runtime(runtime),
    );
    Fixture {
        app: build_router(context),
        _dir: dir,
    }
}

async fn send(
    app: &Router,
    uri: &str,
    host: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, String) {
    let mut builder = Request::builder().uri(uri).header("host", host);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn renders_with_the_active_theme_through_the_runtime() {
    let (runtime, requests) = RecordingRuntime::new();
    let fixture = fixture(Some(Arc::new(runtime)), None).await;

    let (status, body) = send(
        &fixture.app,
        "/products/hoodie?ref=test",
        "shop.example.com",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "<h1>rendered</h1>");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.url, "/products/hoodie?ref=test");
    assert_eq!(request.store.domain, "shop.example.com");
    assert!(!request.request_id.is_empty());
    let theme = request.theme.as_ref().expect("theme context");
    assert_eq!(theme.id, "default");
    assert_eq!(theme.entry, "dist/server.js");
    assert_eq!(theme.settings["brand_color"], "#111111");
}

#[tokio::test]
async fn does_not_render_api_health_or_unknown_host_paths() {
    let (runtime, requests) = RecordingRuntime::new();
    let fixture = fixture(Some(Arc::new(runtime)), None).await;

    let (status, _) = send(&fixture.app, "/api/v1/unknown", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/health/nope", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/", "unknown.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn runtime_timeouts_render_a_gateway_timeout() {
    let fixture = fixture(Some(Arc::new(TimeoutRuntime)), None).await;
    let (status, body) = send(&fixture.app, "/", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"]["code"], "RUNTIME_TIMEOUT");
    assert!(!value["error"]["request_id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn internal_http_surface_requires_a_secret() {
    let fixture = fixture(None, Some("internal-secret".to_owned())).await;

    let (status, _) = send(
        &fixture.app,
        "/internal/storefront/products",
        "shop.example.com",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = send(
        &fixture.app,
        "/internal/storefront/products",
        "shop.example.com",
        &[("x-fastcart-internal-secret", "internal-secret")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn internal_surface_is_not_mounted_without_a_secret() {
    let fixture = fixture(None, None).await;
    let (status, _) = send(
        &fixture.app,
        "/internal/storefront/products",
        "shop.example.com",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
