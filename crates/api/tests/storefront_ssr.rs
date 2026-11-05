//! Storefront SSR fallback and internal bridge security.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::Metadata;
use fastcart_module_cart::{CartService, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateCollectionInput, CreateProductInput};
use fastcart_module_content::{ContentService, CreatePageInput, UpdatePageInput};
use fastcart_module_customer::{CustomerService, RegisterCustomerInput};
use fastcart_module_theme::{RegisterThemeInput, ThemeService};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_runtime_api::{
    RuntimeError, RuntimeMode, RuntimeRequest, RuntimeResponse, StorefrontRuntime,
};
use fastcart_storefront::{ThemeEditorRef, ThemeManifest};
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

struct StreamingRuntime;

#[async_trait::async_trait]
impl StorefrontRuntime for StreamingRuntime {
    async fn render(&self, _request: RuntimeRequest) -> Result<RuntimeResponse, RuntimeError> {
        Ok(RuntimeResponse {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "text/html".to_owned()),
                ("x-stream".to_owned(), "yes".to_owned()),
            ],
            body: fastcart_runtime_api::RuntimeBody::stream(Box::pin(futures::stream::iter([
                Ok::<_, RuntimeError>(b"<html>".to_vec().into()),
                Ok(b"</html>".to_vec().into()),
            ]))),
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
    database: fastcart_infrastructure::StoreDatabase,
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
    let themes = ThemeService::new(database.clone());
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
                editor: Some(ThemeEditorRef {
                    entry: "editor.js".to_owned(),
                    manifest: "editor.manifest.json".to_owned(),
                    schema_version: 1,
                }),
                requires: None,
                routes: Default::default(),
            },
            bundle_path: "themes/default".to_owned(),
            settings_schema: None,
            assets_manifest: None,
            settings: Some(serde_json::json!({"brand_color": "#111111"})),
            editor_manifest: Some(editor_manifest()),
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
        database: database.clone(),
        _dir: dir,
    }
}

/// Editor manifest shared by the test themes.
fn editor_manifest() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "categories": {
            "product": { "title": "Product", "components": ["ProductGrid"] }
        },
        "components": {
            "ProductGrid": {
                "title": "Product Grid",
                "category": "product",
                "fields": {
                    "collection": {
                        "type": "data_source",
                        "source": "collection",
                        "required": true
                    },
                    "limit": { "type": "number", "min": 1, "max": 24, "default": 4 }
                },
                "defaultProps": { "limit": 4 },
                "slots": []
            }
        }
    })
}

/// Provisions one store and activates a theme with the supplied brand color.
async fn provision_with_theme(
    platform: &PlatformStore,
    databases: &Arc<StoreDatabaseManager>,
    name: &str,
    slug: &str,
    host: &str,
    brand_color: &str,
) -> fastcart_infrastructure::StoreDatabase {
    let store = platform
        .create_store(CreateStoreInput::new(name, slug, host))
        .await
        .unwrap();
    StoreProvisioningService::new(platform.clone(), databases.clone())
        .provision(&store.id, None)
        .await
        .unwrap();
    let database = databases.database(&store.id).await.unwrap();
    let themes = ThemeService::new(database.clone());
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
                editor: Some(ThemeEditorRef {
                    entry: "editor.js".to_owned(),
                    manifest: "editor.manifest.json".to_owned(),
                    schema_version: 1,
                }),
                requires: None,
                routes: Default::default(),
            },
            bundle_path: "themes/default".to_owned(),
            settings_schema: None,
            assets_manifest: None,
            settings: Some(serde_json::json!({ "brand_color": brand_color })),
            editor_manifest: Some(editor_manifest()),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();
    database
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
    assert!(request.streaming);
    assert_eq!(request.stream_timeout_ms, 30_000);
}

#[tokio::test]
async fn streams_runtime_bodies_to_the_client() {
    let fixture = fixture(Some(Arc::new(StreamingRuntime)), None).await;

    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-stream")
            .and_then(|value| value.to_str().ok()),
        Some("yes")
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<html></html>");
}

#[tokio::test]
async fn injects_the_published_page_and_resolved_data() {
    let (runtime, requests) = RecordingRuntime::new();
    let fixture = fixture(Some(Arc::new(runtime)), None).await;
    let page = ContentService::new(fixture.database.clone())
        .create_page(CreatePageInput {
            title: "Home".to_owned(),
            handle: Some("home".to_owned()),
            page_type: Some("home".to_owned()),
            body: serde_json::json!({
                "schema_version": 1,
                "content": [{
                    "type": "ProductGrid",
                    "props": {
                        "id": "grid-1",
                        "collection": {
                            "id": "col_missing",
                            "handle": "featured",
                            "label": "Featured"
                        },
                        "limit": 4
                    }
                }],
                "root": { "props": {} },
                "zones": {}
            }),
            seo: serde_json::json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    ContentService::new(fixture.database.clone())
        .publish_page(&page.id)
        .await
        .unwrap();

    let (status, body) = send(&fixture.app, "/", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let requests = requests.lock().unwrap();
    let request = &requests[0];
    assert_eq!(request.mode, RuntimeMode::Production);
    assert!(request.capabilities.is_empty());
    let page = request.page.as_ref().expect("page context");
    assert_eq!(page.handle, "home");
    assert_eq!(page.status, "published");
    let data = request.data.as_ref().expect("data bag");
    let context = data.get("context").expect("reserved context entry");
    assert_eq!(context["mode"], "production");
    assert!(context["cart"].is_null());
    assert!(context["customer"].is_null());
    assert_eq!(context["store"]["id"], request.store.store_id.as_str());
    assert_eq!(context["store"]["name"], "Demo");
    assert_eq!(context["store"]["domain"], "shop.example.com");
    assert_eq!(context["store"]["locale"], "en-US");
    assert_eq!(context["store"]["currency"], "USD");
    assert_eq!(context["store"]["timezone"], "UTC");
    assert_eq!(context["theme"]["id"], "default");
    assert_eq!(context["theme"]["version"], "1.0.0");
    assert_eq!(context["theme"]["settings"]["brand_color"], "#111111");
    let grid = data.get("grid-1").expect("resolved block data");
    // A deleted or missing reference degrades to null instead of failing.
    assert!(grid["collection"].is_null());
    assert_eq!(grid["products"], serde_json::json!([]));
}

#[tokio::test]
async fn serves_robots_and_a_published_page_sitemap() {
    let (runtime, requests) = RecordingRuntime::new();
    let fixture = fixture(Some(Arc::new(runtime)), None).await;
    let content = ContentService::new(fixture.database.clone());
    for (title, handle, page_type) in [
        ("Home", "home", "home"),
        ("About", "about", "custom"),
        ("Draft", "draft", "custom"),
    ] {
        let page = content
            .create_page(CreatePageInput {
                title: title.to_owned(),
                handle: Some(handle.to_owned()),
                page_type: Some(page_type.to_owned()),
                body: serde_json::json!({ "schema_version": 1, "content": [] }),
                seo: serde_json::json!({}),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        if handle != "draft" {
            content.publish_page(&page.id).await.unwrap();
        }
    }

    let (status, robots) = send(&fixture.app, "/robots.txt", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    assert!(robots.contains("Sitemap: https://shop.example.com/sitemap.xml"));

    // A draft handle change must not leak into the sitemap: the storefront
    // still serves the published handle.
    let contact = content
        .create_page(CreatePageInput {
            title: "Contact".to_owned(),
            handle: Some("contact".to_owned()),
            page_type: Some("custom".to_owned()),
            body: serde_json::json!({ "schema_version": 1, "content": [] }),
            seo: serde_json::json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    content.publish_page(&contact.id).await.unwrap();
    content
        .update_page(
            &contact.id,
            UpdatePageInput {
                handle: Some("contact-us".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (status, sitemap) = send(&fixture.app, "/sitemap.xml", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{sitemap}");
    assert!(
        sitemap.contains("<loc>https://shop.example.com/</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.contains("<loc>https://shop.example.com/about</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.contains("<loc>https://shop.example.com/contact</loc>"),
        "{sitemap}"
    );
    assert!(!sitemap.contains("contact-us"), "{sitemap}");
    assert!(!sitemap.contains("draft"), "{sitemap}");
    assert!(sitemap.contains("<lastmod>"), "{sitemap}");
    assert!(
        requests.lock().unwrap().is_empty(),
        "static discovery routes must not invoke the runtime"
    );
}

#[tokio::test]
async fn discovery_files_work_without_a_storefront_runtime() {
    let fixture = fixture(None, None).await;
    let (status, robots) = send(&fixture.app, "/robots.txt", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    let (status, sitemap) = send(&fixture.app, "/sitemap.xml", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{sitemap}");
    let (status, _) = send(&fixture.app, "/", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A TLS-terminating proxy can declare the public scheme.
    let (status, robots) = send(
        &fixture.app,
        "/robots.txt",
        "shop.example.com",
        &[("x-forwarded-proto", "http")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{robots}");
    assert!(robots.contains("http://shop.example.com/sitemap.xml"));
}

#[tokio::test]
async fn serves_theme_client_assets_and_rejects_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let themes_root = dir.path().join("themes");
    let bundle = themes_root.join("default");
    tokio::fs::create_dir_all(bundle.join("dist/client"))
        .await
        .unwrap();
    tokio::fs::write(bundle.join("dist/client/client.js"), "console.log('theme')")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("secret.txt"), "do not serve")
        .await
        .unwrap();

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
    let themes = ThemeService::new(databases.database(&store.id).await.unwrap());
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
                editor: None,
                requires: None,
                routes: Default::default(),
            },
            bundle_path: "default".to_owned(),
            settings_schema: None,
            assets_manifest: Some(serde_json::json!({
                "entries": { "client": "dist/client" }
            })),
            settings: None,
            editor_manifest: None,
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let app = build_router(Arc::new(AppContext::new(
        ApiConfig {
            cookie_secure: false,
            backup_dir: dir.path().join("backups"),
            themes_root,
            ..ApiConfig::default()
        },
        platform,
        databases,
        StoreCache::default_capacity(),
    )));

    let (status, body) = send(
        &app,
        "/api/v1/store/theme-assets/client.js",
        "shop.example.com",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "console.log('theme')");

    let (status, _) = send(
        &app,
        "/api/v1/store/theme-assets/missing.js",
        "shop.example.com",
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Encoded traversal must not escape the bundle's client directory.
    for path in [
        "/api/v1/store/theme-assets/%2e%2e/%2e%2e/secret.txt",
        "/api/v1/store/theme-assets/..%2f..%2fsecret.txt",
    ] {
        let (status, _) = send(&app, path, "shop.example.com", &[]).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn renders_theme_dynamic_routes_from_published_templates() {
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
    let themes = ThemeService::new(database.clone());
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
                editor: Some(ThemeEditorRef {
                    entry: "editor.js".to_owned(),
                    manifest: "editor.manifest.json".to_owned(),
                    schema_version: 1,
                }),
                requires: None,
                routes: [
                    ("product".to_owned(), "/products/{handle}".to_owned()),
                    ("collection".to_owned(), "/collections/{handle}".to_owned()),
                ]
                .into_iter()
                .collect(),
            },
            bundle_path: "default".to_owned(),
            settings_schema: None,
            assets_manifest: None,
            settings: None,
            editor_manifest: Some(serde_json::json!({
                "schema_version": 1,
                "categories": {
                    "commerce": {
                        "title": "Commerce",
                        "components": ["ProductDetail", "CollectionDetail"]
                    }
                },
                "components": {
                    "CollectionDetail": {
                        "title": "Collection detail",
                        "category": "commerce",
                        "fields": {},
                        "defaultProps": {},
                        "slots": []
                    },
                    "ProductDetail": {
                        "title": "Product detail",
                        "category": "commerce",
                        "fields": {
                            "emptyText": { "type": "text" }
                        },
                        "defaultProps": {},
                        "slots": []
                    }
                }
            })),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let content = ContentService::new(database.clone());
    let template = content
        .create_page(CreatePageInput {
            title: "Product".to_owned(),
            handle: Some("product-template".to_owned()),
            page_type: Some("product".to_owned()),
            body: serde_json::json!({
                "schema_version": 1,
                "content": [{
                    "type": "ProductDetail",
                    "props": { "id": "pd-1" }
                }],
                "root": { "props": {} },
                "zones": {}
            }),
            seo: serde_json::json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    content.publish_page(&template.id).await.unwrap();

    let catalog = CatalogService::new(database.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Hoodie".to_owned(),
            handle: Some("hoodie".to_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();

    let collection = catalog
        .create_collection(CreateCollectionInput {
            title: "Featured".to_owned(),
            handle: Some("featured".to_owned()),
            is_published: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    catalog
        .add_product_to_collection(&product.id, &collection.id, 0)
        .await
        .unwrap();

    let collection_template = content
        .create_page(CreatePageInput {
            title: "Collection".to_owned(),
            handle: Some("collection-template".to_owned()),
            page_type: Some("collection".to_owned()),
            body: serde_json::json!({
                "schema_version": 1,
                "content": [{
                    "type": "CollectionDetail",
                    "props": { "id": "cd-1" }
                }],
                "root": { "props": {} },
                "zones": {}
            }),
            seo: serde_json::json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    content.publish_page(&collection_template.id).await.unwrap();

    let (runtime, requests) = RecordingRuntime::new();
    let app = build_router(Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_storefront_runtime(Some(Arc::new(runtime))),
    ));

    let (status, body) = send(&app, "/products/hoodie", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    {
        let requests = requests.lock().unwrap();
        let request = &requests[0];
        let page = request.page.as_ref().expect("route template page");
        assert_eq!(page.page_type, "product");
        assert_eq!(page.handle, "product-template");
        let data = request.data.as_ref().expect("data bag");
        let route = &data["context"]["route"];
        assert_eq!(route["kind"], "product");
        assert_eq!(route["handle"], "hoodie");
        assert_eq!(route["resource"]["product"]["title"], "Hoodie");
    }

    let (status, body) = send(&app, "/collections/featured", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    {
        let requests = requests.lock().unwrap();
        let request = &requests[1];
        let page = request.page.as_ref().expect("collection template page");
        assert_eq!(page.page_type, "collection");
        let data = request.data.as_ref().expect("data bag");
        let route = &data["context"]["route"];
        assert_eq!(route["kind"], "collection");
        assert_eq!(route["resource"]["collection"]["title"], "Featured");
        assert_eq!(
            route["resource"]["products"][0]["title"], "Hoodie",
            "collection products are resolved for the route"
        );
    }

    // An unknown handle resolves no route resource, so the runtime receives no
    // page and the theme renders its own not-found document.
    let (status, body) = send(&app, "/products/missing", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        requests.lock().unwrap()[2].page.is_none(),
        "missing resources must not render the template"
    );

    let (status, sitemap) = send(&app, "/sitemap.xml", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{sitemap}");
    assert!(
        sitemap.contains("<loc>https://shop.example.com/products/hoodie</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.contains("<loc>https://shop.example.com/collections/featured</loc>"),
        "{sitemap}"
    );
    assert!(
        sitemap.matches("<lastmod>").count() >= 2,
        "route URLs carry lastmod: {sitemap}"
    );
}

#[tokio::test]
async fn injects_customer_and_cart_context_from_cookies() {
    let (runtime, requests) = RecordingRuntime::new();
    let fixture = fixture(Some(Arc::new(runtime)), None).await;
    let customers = CustomerService::new(fixture.database.clone());
    let customer = customers
        .register(RegisterCustomerInput::new(
            "shopper@example.com",
            "correct-horse-battery-staple",
        ))
        .await
        .expect("customer");
    let session = customers
        .login("shopper@example.com", "correct-horse-battery-staple")
        .await
        .expect("customer session");
    let cart = CartService::new(fixture.database.clone())
        .create_cart(CreateCartInput {
            customer_id: Some(customer.id.clone()),
            ..CreateCartInput::new("USD")
        })
        .await
        .expect("cart");
    let cookie = format!(
        "fc_customer_session={}; fc_cart_id={}",
        session.token,
        cart.id.as_str()
    );

    let (status, body) = send(
        &fixture.app,
        "/",
        "shop.example.com",
        &[("cookie", cookie.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let requests = requests.lock().unwrap();
    let data = requests[0].data.as_ref().expect("data bag");
    let context = data.get("context").expect("reserved context entry");
    assert_eq!(context["customer"]["id"], customer.id.as_str());
    assert_eq!(context["customer"]["email"], "shopper@example.com");
    assert_eq!(context["cart"]["id"], cart.id.as_str());
    assert_eq!(context["cart"]["item_count"], 0);
    assert_eq!(context["cart"]["currency"], "USD");
}

#[tokio::test]
async fn compiled_pages_and_themes_do_not_leak_between_stores() {
    let (runtime, requests) = RecordingRuntime::new();
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let alpha_database = provision_with_theme(
        &platform,
        &databases,
        "Alpha",
        "alpha",
        "alpha.example.com",
        "#111111",
    )
    .await;
    let beta_database = provision_with_theme(
        &platform,
        &databases,
        "Beta",
        "beta",
        "beta.example.com",
        "#222222",
    )
    .await;

    for (database, block_id, collection_id) in [
        (&alpha_database, "grid-a", "col_a"),
        (&beta_database, "grid-b", "col_b"),
    ] {
        let page = ContentService::new(database.clone())
            .create_page(CreatePageInput {
                title: "Home".to_owned(),
                handle: Some("home".to_owned()),
                page_type: Some("home".to_owned()),
                body: serde_json::json!({
                    "schema_version": 1,
                    "content": [{
                        "type": "ProductGrid",
                        "props": {
                            "id": block_id,
                            "collection": {
                                "id": collection_id,
                                "handle": collection_id,
                                "label": collection_id
                            },
                            "limit": 4
                        }
                    }],
                    "root": { "props": {} },
                    "zones": {}
                }),
                seo: serde_json::json!({}),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap();
        ContentService::new(database.clone())
            .publish_page(&page.id)
            .await
            .unwrap();
    }

    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_storefront_runtime(Some(Arc::new(runtime))),
    );
    let app = build_router(context);

    // Render beta first so an editor cache keyed without the store would be
    // observable when alpha renders.
    let (status, body) = send(&app, "/", "beta.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&app, "/", "alpha.example.com", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let alpha = requests
        .iter()
        .find(|request| request.store.domain == "alpha.example.com")
        .expect("alpha render");
    let beta = requests
        .iter()
        .find(|request| request.store.domain == "beta.example.com")
        .expect("beta render");

    let alpha_data = alpha.data.as_ref().expect("alpha data bag");
    assert!(alpha_data.get("grid-a").is_some());
    assert!(alpha_data.get("grid-b").is_none());
    assert_eq!(alpha_data["context"]["store"]["name"], "Alpha");
    assert_eq!(
        alpha_data["context"]["theme"]["settings"]["brand_color"],
        "#111111"
    );

    let beta_data = beta.data.as_ref().expect("beta data bag");
    assert!(beta_data.get("grid-b").is_some());
    assert!(beta_data.get("grid-a").is_none());
    assert_eq!(beta_data["context"]["store"]["name"], "Beta");
    assert_eq!(
        beta_data["context"]["theme"]["settings"]["brand_color"],
        "#222222"
    );

    assert_eq!(
        alpha_data["context"]["store"]["id"],
        alpha.store.store_id.as_str()
    );
    assert_eq!(
        beta_data["context"]["store"]["id"],
        beta.store.store_id.as_str()
    );
    assert_ne!(alpha.store.store_id, beta.store.store_id);
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
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(value["products"].as_array().is_some());
    assert_eq!(value["count"], 0);
    assert_eq!(value["offset"], 0);
    assert!(value["limit"].as_i64().is_some());
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
