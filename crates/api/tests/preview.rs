//! Draft page preview flow: token issuance and universal preview rendering.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use fastcart_api::routes::preview::issue_preview_token;
use fastcart_api::state::UniversalStorefront;
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreContext, StoreId, now_timestamp};
use fastcart_module_content::{ContentService, CreatePageInput};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";
const PREVIEW_SECRET: &str = "preview-integration-secret-0123456789abcdef";

#[derive(Default)]
struct RecordingUniversal {
    calls: Mutex<Vec<PreviewCall>>,
}

struct PreviewCall {
    uri: String,
    host: Option<String>,
    store: StoreContext,
    request_id: Option<String>,
    scheme: String,
}

#[async_trait::async_trait]
impl UniversalStorefront for RecordingUniversal {
    async fn render(
        &self,
        _request: Request<Body>,
        _store: StoreContext,
        _request_id: Option<String>,
        _scheme: String,
    ) -> Response {
        StatusCode::NOT_FOUND.into_response()
    }

    async fn serve_asset(
        &self,
        _store_id: &str,
        _path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        StatusCode::NOT_FOUND.into_response()
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
        request: Request<Body>,
        store: StoreContext,
        request_id: Option<String>,
        scheme: String,
    ) -> Response {
        self.calls.lock().unwrap().push(PreviewCall {
            uri: request.uri().to_string(),
            host: request
                .headers()
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            store,
            request_id,
            scheme,
        });
        (StatusCode::OK, "<h1>universal-preview</h1>").into_response()
    }

    async fn has_preview(&self, _store_id: &str) -> bool {
        true
    }

    async fn serve_preview_asset(
        &self,
        _store_id: &str,
        _path: String,
        _headers: axum::http::HeaderMap,
    ) -> Response {
        StatusCode::NOT_FOUND.into_response()
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
    store_id: String,
    page_id: String,
    session: String,
    csrf: String,
    universal: Option<Arc<RecordingUniversal>>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn admin_cookie(&self) -> String {
        format!("fc_admin_session={}; fc_csrf={}", self.session, self.csrf)
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, Vec<PreviewCall>> {
        self.universal
            .as_ref()
            .expect("fixture has a universal storefront")
            .calls
            .lock()
            .unwrap()
    }
}

async fn fixture(preview_secret: Option<&str>, universal: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let universal = universal.then(|| Arc::new(RecordingUniversal::default()));
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
        .with_universal_storefront(
            universal
                .clone()
                .map(|runtime| runtime as Arc<dyn UniversalStorefront>),
        ),
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
    let page = ContentService::new(database.clone())
        .create_page(CreatePageInput {
            title: "About".to_owned(),
            handle: Some("about".to_owned()),
            page_type: Some("custom".to_owned()),
            body: json!({ "schema_version": 1, "content": [] }),
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
        universal,
        _dir: dir,
    }
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
    send_preview_with_host(app, uri, HOST).await
}

async fn send_preview_with_host(app: &Router, uri: &str, host: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", host)
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

async fn issue_token(fixture: &Fixture) -> (String, i64, String) {
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
    let jti = body["jti"].as_str().expect("token identifier").to_owned();
    (token, expires_at, jti)
}

#[tokio::test]
async fn preview_token_renders_through_the_universal_deployment() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
    let (token, expires_at, jti) = issue_token(&fixture).await;
    assert!(expires_at > now_timestamp());
    assert_eq!(token.matches('.').count(), 1);
    assert_eq!(jti.len(), 32);

    let uri = format!("/preview/about?token={token}&x=1");
    let (status, body) = send_preview(&fixture.app, &uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, "<h1>universal-preview</h1>");

    let calls = fixture.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.uri, "/about?x=1", "the preview token is stripped");
    assert_eq!(call.host.as_deref(), Some(HOST));
    assert_eq!(call.store.domain, HOST);
    assert!(call.request_id.is_some());
    assert_eq!(call.scheme, "https");
}

#[tokio::test]
async fn preview_renders_on_an_admin_host() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
    let (token, _, _) = issue_token(&fixture).await;

    // The admin is served from its own domain; the signed token names the
    // store, so the iframe works without the Host matching the storefront.
    let (status, body) = send_preview_with_host(
        &fixture.app,
        &format!("/preview/about?token={token}"),
        "admin.example.com",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let calls = fixture.calls();
    assert_eq!(calls[0].store.domain, HOST);
    assert_eq!(calls[0].host.as_deref(), Some(HOST));
}

#[tokio::test]
async fn revoked_tokens_stop_rendering_immediately() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
    let (token, _, jti) = issue_token(&fixture).await;
    let admin_cookie = fixture.admin_cookie();
    let (status, body, _) = request(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/content/pages/{}/preview-token/revoke",
            fixture.page_id
        ),
        Some(json!({ "jti": jti })),
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, body) = send_preview(&fixture.app, &format!("/preview/about?token={token}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(
        fixture.calls().is_empty(),
        "revoked tokens must not reach the runtime"
    );

    // Revocation is per token: a newly issued preview still renders.
    let (fresh, _, _) = issue_token(&fixture).await;
    let (status, body) = send_preview(&fixture.app, &format!("/preview/about?token={fresh}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(fixture.calls().len(), 1);
}

#[tokio::test]
async fn invalid_tokens_are_rejected_without_invoking_the_runtime() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
    let (valid, _, _) = issue_token(&fixture).await;

    let (payload, signature) = valid.split_once('.').expect("token shape");
    let mut bytes = signature.as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
    let tampered = format!("{payload}.{}", String::from_utf8(bytes).unwrap());
    let expired = issue_preview_token(PREVIEW_SECRET, &fixture.store_id, &fixture.page_id, 0)
        .unwrap()
        .token;
    let wrong_store = issue_preview_token(
        PREVIEW_SECRET,
        "store_other",
        &fixture.page_id,
        now_timestamp(),
    )
    .unwrap()
    .token;

    for token in [tampered, expired, "malformed".to_owned()] {
        let uri = format!("/preview/about?token={token}");
        let (status, body) = send_preview(&fixture.app, &uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    }
    // A signed token for a store that does not exist resolves to that store id;
    // it must not fall back to the Host header.
    let (status, body) =
        send_preview(&fixture.app, &format!("/preview/about?token={wrong_store}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = send_preview(&fixture.app, "/preview/about").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn preview_is_disabled_without_a_secret() {
    let fixture = fixture(None, true).await;
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
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn preview_without_a_universal_deployment_returns_not_found() {
    let fixture = fixture(Some(PREVIEW_SECRET), false).await;
    let (token, _, _) = issue_token(&fixture).await;

    let (status, _) = send_preview(&fixture.app, &format!("/preview/about?token={token}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_page_returns_not_found() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
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
async fn archived_pages_cannot_issue_preview_tokens() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
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
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn preview_token_response_is_not_cacheable() {
    let fixture = fixture(Some(PREVIEW_SECRET), true).await;
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
