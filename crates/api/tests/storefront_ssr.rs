//! Storefront SSR fallback and internal bridge security.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use fastcart_api::routes::preview::issue_preview_token;
use fastcart_api::state::{InternalSsrCredentials, InternalSsrToken, UniversalStorefront};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreContext, now_timestamp};
use fastcart_module_content::{ContentService, CreatePageInput, UpdatePageInput};
use fastcart_module_theme::{RegisterThemeInput, ThemeService};
use fastcart_platform::{CreateStoreInput, PlatformStore, StoreProvisioningService};
use fastcart_storefront::ThemeManifest;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[derive(Default)]
struct RecordingUniversal {
    seen: Arc<Mutex<Vec<RecordedContext>>>,
    preview: bool,
}

struct RecordedContext {
    store: StoreContext,
    request_id: Option<String>,
    scheme: String,
}

#[async_trait::async_trait]
impl UniversalStorefront for RecordingUniversal {
    async fn render(
        &self,
        _request: Request<Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        self.seen.lock().unwrap().push(RecordedContext {
            store,
            request_id,
            scheme,
        });
        (StatusCode::OK, "<h1>universal</h1>").into_response()
    }

    async fn serve_asset(
        &self,
        store_id: &str,
        path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        (StatusCode::OK, format!("asset:{store_id}:{path}")).into_response()
    }

    async fn activate(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn rollback(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn render_preview(
        &self,
        _request: Request<Body>,
        _store: StoreContext,
        _request_id: Option<String>,
        _scheme: String,
    ) -> Response {
        (StatusCode::OK, "<h1>universal-preview</h1>").into_response()
    }

    async fn has_preview(&self, _store_id: &str) -> bool {
        self.preview
    }

    async fn serve_preview_asset(
        &self,
        store_id: &str,
        path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        (StatusCode::OK, format!("preview-asset:{store_id}:{path}")).into_response()
    }

    async fn activate_preview(
        &self,
        _store_id: &str,
        _deployment_id: &str,
        _artifact_path: String,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn deactivate_preview(&self, _store_id: &str) -> Result<(), String> {
        Ok(())
    }
}

struct Fixture {
    app: Router,
    database: fastcart_infrastructure::StoreDatabase,
    platform: PlatformStore,
    preview_asset_authorizations: moka::future::Cache<String, ()>,
    store_id: String,
    _dir: tempfile::TempDir,
}

async fn fixture(universal: Option<Arc<dyn UniversalStorefront>>) -> Fixture {
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
            },
            settings_schema: None,
            settings: Some(serde_json::json!({"brand_color": "#111111"})),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let fixture_platform = platform.clone();
    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                internal_ssr_token: Some("ssr-internal-secret".to_owned()),
                internal_ssr_credentials: InternalSsrCredentials::new(vec![
                    InternalSsrToken {
                        token: "ssr-internal-secret".to_owned(),
                        store_id: Some(store.id.to_string()),
                        expires_at: None,
                        preview: false,
                    },
                    InternalSsrToken {
                        token: "other-store-secret".to_owned(),
                        store_id: Some("store_other".to_owned()),
                        expires_at: None,
                        preview: false,
                    },
                    InternalSsrToken {
                        token: "preview-store-secret".to_owned(),
                        store_id: Some(store.id.to_string()),
                        expires_at: None,
                        preview: true,
                    },
                ]),
                preview_secret: Some("preview-secret-that-is-at-least-32-bytes".to_owned()),
                ..ApiConfig::default()
            },
            platform,
            databases,
            StoreCache::default_capacity(),
        )
        .with_universal_storefront(universal),
    );
    Fixture {
        app: build_router(context.clone()),
        database: database.clone(),
        platform: fixture_platform,
        preview_asset_authorizations: context.preview_asset_authorizations.clone(),
        store_id: store.id.to_string(),
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
async fn serves_robots_and_a_published_page_sitemap() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;
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
        universal.seen.lock().unwrap().is_empty(),
        "static discovery routes must not invoke the runtime"
    );
}

#[tokio::test]
async fn discovery_files_work_without_ssr() {
    let fixture = fixture(None).await;
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
async fn does_not_render_api_health_or_unknown_host_paths() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;

    let (status, _) = send(&fixture.app, "/api/v1/unknown", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/health/nope", "shop.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&fixture.app, "/", "unknown.example.com", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(universal.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn universal_runtime_receives_trusted_context() {
    let universal = Arc::new(RecordingUniversal::default());
    let fixture = fixture(Some(universal.clone())).await;
    let request = Request::builder()
        .uri("/products/example")
        .header("host", "shop.example.com")
        .header("x-fastcart-store-id", "attacker")
        .body(Body::empty())
        .unwrap();
    let response = fixture.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<h1>universal</h1>");
    let seen = universal.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].store.store_id.as_str().starts_with("store_"));
    assert!(seen[0].request_id.is_some());
    assert_eq!(seen[0].scheme, "https");
}

#[tokio::test]
async fn internal_ssr_namespace_requires_token_and_uses_path_store() {
    let fixture = fixture(None).await;
    let context_path = format!("/internal/ssr/v1/stores/{}/context", fixture.store_id);

    let authorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    let body = authorized.into_body().collect().await.unwrap().to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["store_id"], fixture.store_id);

    let rejected = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);

    let unknown = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/internal/ssr/v1/stores/store_missing/context")
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let cross_store = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer other-store-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_store.status(), StatusCode::FORBIDDEN);

    let preview_read = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&context_path)
                .header("authorization", "Bearer preview-store-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_read.status(), StatusCode::OK);

    let preview_write = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/carts",
                    fixture.store_id
                ))
                .header("authorization", "Bearer preview-store-secret")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_write.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn universal_assets_are_served_without_store_resolution() {
    let universal = Arc::new(RecordingUniversal {
        preview: true,
        ..RecordingUniversal::default()
    });
    let active = fixture(Some(universal)).await;
    let preview_token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        &active.store_id,
        "page_1",
        now_timestamp(),
    )
    .expect("preview token");
    let preview_token_value = preview_token.token.clone();
    let preview_jti = preview_token.jti;
    let response = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/assets/assets/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"asset:store_"));
    assert!(body.ends_with(b":assets/app.js"));

    let sveltekit_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_app/immutable/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(sveltekit_asset.status(), StatusCode::OK);
    let body = sveltekit_asset
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(body.ends_with(b":_app/immutable/app.js"));

    let preview_page = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/?token={preview_token_value}"))
                .header("host", "shop.example.com")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_page.status(), StatusCode::OK);
    let set_cookie = preview_page
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let value = value.to_str().ok()?;
            value.starts_with("fastcart_preview=").then_some(value)
        })
        .expect("preview asset cookie")
        .to_owned();
    assert!(set_cookie.contains("Path=/_fastcart/preview-assets"));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("Secure"));
    assert!(set_cookie.contains("SameSite=None"));
    assert!(set_cookie.contains("Partitioned"));
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();

    let http_preview = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/?token={preview_token_value}"))
                .header("host", "shop.example.com")
                .header("x-forwarded-proto", "http")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let http_cookie = http_preview
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            let value = value.to_str().ok()?;
            value.starts_with("fastcart_preview=").then_some(value)
        })
        .expect("http preview cookie");
    assert!(http_cookie.contains("SameSite=Lax"));
    assert!(!http_cookie.contains("Secure"));
    assert!(!http_cookie.contains("Partitioned"));

    let unauthenticated = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::FORBIDDEN);

    let preview_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_asset.status(), StatusCode::OK);
    assert_eq!(
        preview_asset
            .headers()
            .get("vary")
            .and_then(|value| value.to_str().ok()),
        Some("Cookie")
    );

    let other_store_token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        "store_other",
        "page_1",
        now_timestamp(),
    )
    .expect("other store token")
    .token;
    let wrong_store = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", format!("fastcart_preview={other_store_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong_store.status(), StatusCode::FORBIDDEN);

    sqlx::query(
        "INSERT OR REPLACE INTO preview_token_revocations (jti, page_id, expires_at) VALUES (?, ?, ?)",
    )
    .bind(&preview_jti)
    .bind("page_1")
    .bind(now_timestamp() + 900)
    .execute(active.database.pool())
    .await
    .expect("revoke preview token");
    active
        .preview_asset_authorizations
        .invalidate(&format!("{}:{preview_jti}", active.store_id))
        .await;
    let revoked_asset = active
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/preview-assets/app.js")
                .header("host", "shop.example.com")
                .header("cookie", format!("fastcart_preview={preview_token_value}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked_asset.status(), StatusCode::FORBIDDEN);

    let body = preview_asset
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert!(body.starts_with(b"preview-asset:store_"));
    assert!(body.ends_with(b":app.js"));

    let no_runtime = fixture(None).await;
    let missing = no_runtime
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_fastcart/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deployment_lifecycle_requires_the_internal_token() {
    let fixture = fixture(Some(Arc::new(RecordingUniversal::default()))).await;
    let activate_path = format!("/internal/ssr/v1/deployments/{}/activate", fixture.store_id);
    let body = serde_json::json!({
        "deployment_id": "deploy_next",
        "artifact_path": "/tmp/next-artifact"
    });

    let unauthorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&activate_path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);

    let authorized = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&activate_path)
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::NO_CONTENT);

    let rollback = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rollback",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rollback.status(), StatusCode::NO_CONTENT);

    let preview = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/preview/activate",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::NO_CONTENT);

    let deactivate = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/preview/deactivate",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deactivate.status(), StatusCode::NO_CONTENT);

    let rotate_body = serde_json::json!({
        "token": "rotated-token-0123456789",
        "grace_seconds": 300
    });
    let rotated = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rotate-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(rotate_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rotated.status(), StatusCode::NO_CONTENT);

    let new_token = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/context",
                    fixture.store_id
                ))
                .header("authorization", "Bearer rotated-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_token.status(), StatusCode::OK);

    let revoked = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/revoke-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"token": "rotated-token-0123456789"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);

    let after_revoke = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/internal/ssr/v1/stores/{}/context",
                    fixture.store_id
                ))
                .header("authorization", "Bearer rotated-token-0123456789")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after_revoke.status(), StatusCode::FORBIDDEN);

    let expired = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/ssr/v1/deployments/{}/rotate-token",
                    fixture.store_id
                ))
                .header("authorization", "Bearer ssr-internal-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "token": "another-token-0123456789",
                        "expires_at": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::BAD_REQUEST);

    let logs = fixture
        .platform
        .list_audit_logs(
            Some(&fastcart_kernel::StoreId::from_string(
                fixture.store_id.clone(),
            )),
            fastcart_kernel::Pagination {
                limit: 20,
                offset: 0,
            },
        )
        .await
        .expect("audit logs");
    for action in [
        "ssr.deployment.activate",
        "ssr.deployment.rollback",
        "ssr.preview.activate",
        "ssr.preview.deactivate",
        "ssr.credential.rotate",
        "ssr.credential.revoke",
    ] {
        assert!(
            logs.iter().any(|entry| entry.action == action),
            "missing audit action {action}"
        );
    }
}

#[tokio::test]
async fn preview_route_uses_the_universal_preview_deployment() {
    let universal = Arc::new(RecordingUniversal {
        preview: true,
        ..RecordingUniversal::default()
    });
    let fixture = fixture(Some(universal)).await;
    let token = issue_preview_token(
        "preview-secret-that-is-at-least-32-bytes",
        &fixture.store_id,
        "page_test",
        now_timestamp(),
    )
    .expect("preview token");
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/preview/products?token={}", token.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"<h1>universal-preview</h1>");
}
