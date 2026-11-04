//! Draft page preview flow: token issuance and preview rendering.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::routes::preview::issue_preview_token;
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreId, now_timestamp};
use fastcart_module_content::{ContentService, CreatePageInput};
use fastcart_module_theme::{RegisterThemeInput, ThemeService};
use fastcart_platform::PlatformStore;
use fastcart_runtime_api::{
    RuntimeError, RuntimeMode, RuntimeRequest, RuntimeResponse, StorefrontRuntime,
};
use fastcart_storefront::{ThemeEditorRef, ThemeManifest};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";
const PREVIEW_SECRET: &str = "preview-integration-secret-0123456789abcdef";

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
            body: fastcart_runtime_api::RuntimeBody::full("<h1>preview</h1>"),
        })
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
    store_id: String,
    page_id: String,
    session: String,
    csrf: String,
    requests: Arc<Mutex<Vec<RuntimeRequest>>>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn admin_cookie(&self) -> String {
        format!("fc_admin_session={}; fc_csrf={}", self.session, self.csrf)
    }
}

async fn fixture(preview_secret: Option<&str>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let (runtime, requests) = RecordingRuntime::new();
    let context = Arc::new(
        AppContext::new(
            ApiConfig {
                cookie_secure: false,
                backup_dir: dir.path().join("backups"),
                preview_secret: preview_secret.map(str::to_owned),
                ..ApiConfig::default()
            },
            platform,
            databases.clone(),
            StoreCache::default_capacity(),
        )
        .with_storefront_runtime(Some(Arc::new(runtime))),
    );
    let app = build_router(context);

    let setup = request(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(json!({
            "email": "owner@example.com",
            "password": "admin-pass-123"
        })),
        &[],
    )
    .await;
    assert_eq!(setup.0, StatusCode::OK, "{}", setup.1);
    let session = cookie_value(&setup.2, "fc_admin_session").expect("admin session cookie");
    let csrf = cookie_value(&setup.2, "fc_csrf").expect("csrf cookie");
    let admin_cookie = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let store = request(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": HOST
        })),
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(store.0, StatusCode::CREATED, "{}", store.1);
    let store_id = store.1["id"].as_str().unwrap().to_owned();

    let database = databases
        .database(&StoreId::from_string(store_id.clone()))
        .await
        .unwrap();
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
            },
            bundle_path: "themes/default".to_owned(),
            settings_schema: None,
            settings: Some(json!({ "brand_color": "#111111" })),
            editor_manifest: Some(editor_manifest()),
        })
        .await
        .unwrap();
    themes.activate(&theme.id).await.unwrap();

    let page = ContentService::new(database)
        .create_page(CreatePageInput {
            title: "About".to_owned(),
            handle: Some("about".to_owned()),
            page_type: Some("custom".to_owned()),
            body: page_body(),
            seo: json!({}),
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();

    Fixture {
        app,
        store_id,
        page_id: page.id.as_str().to_owned(),
        session,
        csrf,
        requests,
        _dir: dir,
    }
}

fn editor_manifest() -> Value {
    json!({
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

fn page_body() -> Value {
    json!({
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
    })
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body, cookies)
}

async fn send_preview(app: &Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

async fn issue_token(fixture: &Fixture) -> (String, i64) {
    let admin_cookie = fixture.admin_cookie();
    let (status, body, _) = request(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/content/pages/{}/preview-token",
            fixture.page_id
        ),
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["token"].as_str().expect("preview token").to_owned();
    let expires_at = body["expires_at"].as_i64().expect("expiry");
    (token, expires_at)
}

#[tokio::test]
async fn preview_token_renders_the_draft_page_through_the_runtime() {
    let fixture = fixture(Some(PREVIEW_SECRET)).await;
    let (token, expires_at) = issue_token(&fixture).await;
    assert!(expires_at > now_timestamp());
    assert_eq!(token.matches('.').count(), 1);

    let uri = format!("/preview/about?token={token}&x=1");
    let (status, body) = send_preview(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "<h1>preview</h1>");

    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.mode, RuntimeMode::Preview);
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "/about?x=1");
    assert_eq!(request.store.domain, HOST);
    let page = request.page.as_ref().expect("page context");
    assert_eq!(page.id, fixture.page_id);
    assert_eq!(page.handle, "about");
    assert_eq!(page.status, "draft");
    assert_eq!(page.body["content"][0]["type"], "ProductGrid");
    let data = request.data.as_ref().expect("data bag");
    let context = data.get("context").expect("reserved context entry");
    assert_eq!(context["mode"], "preview");
    assert_eq!(context["store"]["id"], fixture.store_id);
    assert!(data.get("grid-1").is_some(), "resolved binding data");
}

#[tokio::test]
async fn invalid_tokens_are_rejected_without_invoking_the_runtime() {
    let fixture = fixture(Some(PREVIEW_SECRET)).await;
    let (valid, _) = issue_token(&fixture).await;

    let (payload, signature) = valid.split_once('.').expect("token shape");
    let mut bytes = signature.as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
    let tampered = format!("{payload}.{}", String::from_utf8(bytes).unwrap());
    let (expired, _) =
        issue_preview_token(PREVIEW_SECRET, &fixture.store_id, &fixture.page_id, 0).unwrap();
    let (wrong_store, _) = issue_preview_token(
        PREVIEW_SECRET,
        "store_other",
        &fixture.page_id,
        now_timestamp(),
    )
    .unwrap();

    for token in [tampered, expired, wrong_store, "malformed".to_owned()] {
        let uri = format!("/preview/about?token={token}");
        let (status, body) = send_preview(&fixture.app, &uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }
    let (status, body) = send_preview(&fixture.app, "/preview/about").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preview_is_disabled_without_a_secret() {
    let fixture = fixture(None).await;
    let admin_cookie = fixture.admin_cookie();
    let (status, body, _) = request(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/content/pages/{}/preview-token",
            fixture.page_id
        ),
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");

    let (status, body) = send_preview(&fixture.app, "/preview/about?token=anything").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{body}");
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_page_returns_not_found() {
    let fixture = fixture(Some(PREVIEW_SECRET)).await;
    let admin_cookie = fixture.admin_cookie();
    let (status, body, _) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/content/pages/page_missing/preview-token",
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn archived_pages_cannot_be_previewed() {
    let fixture = fixture(Some(PREVIEW_SECRET)).await;
    let admin_cookie = fixture.admin_cookie();
    let (status, body, _) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/content/pages/{}/archive", fixture.page_id),
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body, _) = request(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/content/pages/{}/preview-token",
            fixture.page_id
        ),
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // A token issued before archiving must stop rendering too.
    let (token, _) = issue_preview_token(
        PREVIEW_SECRET,
        &fixture.store_id,
        &fixture.page_id,
        now_timestamp(),
    )
    .unwrap();
    let (status, body) = send_preview(&fixture.app, &format!("/preview/about?token={token}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(fixture.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn preview_token_response_is_not_cacheable() {
    let fixture = fixture(Some(PREVIEW_SECRET)).await;
    let admin_cookie = fixture.admin_cookie();
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/admin/content/pages/{}/preview-token",
                    fixture.page_id
                ))
                .header("cookie", admin_cookie)
                .header("x-csrf-token", fixture.csrf)
                .header("x-store-id", fixture.store_id)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("cache-control"),
        Some(&axum::http::HeaderValue::from_static("no-store"))
    );
}
