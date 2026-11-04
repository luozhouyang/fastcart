//! Admin media library and public asset serving flow.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake-png-bytes";

struct Fixture {
    app: Router,
    store_id: String,
    session: String,
    csrf: String,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn admin_cookie(&self) -> String {
        format!("fc_admin_session={}; fc_csrf={}", self.session, self.csrf)
    }
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let context = Arc::new(AppContext::new(
        ApiConfig {
            cookie_secure: false,
            backup_dir: dir.path().join("backups"),
            storage_root: dir.path().join("assets"),
            ..ApiConfig::default()
        },
        platform,
        databases.clone(),
        StoreCache::default_capacity(),
    ));
    let app = build_router(context);

    let setup = request(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        None,
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
        None,
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
    databases
        .database(&StoreId::from_string(store_id.clone()))
        .await
        .unwrap();

    Fixture {
        app,
        store_id,
        session,
        csrf,
        _dir: dir,
    }
}

fn multipart_body(boundary: &str, filename: &str, alt: Option<&str>) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(PNG);
    body.extend_from_slice(b"\r\n");
    if let Some(alt) = alt {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"alt\"\r\n\r\n{alt}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", content_type.unwrap_or("application/json"));
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

async fn raw_request(
    app: &Router,
    method: &str,
    uri: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
    headers: &[(&str, &str)],
) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = content_type {
        builder = builder.header("content-type", value);
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec(), headers)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

#[tokio::test]
async fn upload_list_serve_and_delete_an_asset() {
    let fixture = fixture().await;
    let admin_cookie = fixture.admin_cookie();
    let boundary = "fastcart-test-boundary";

    let (status, bytes, _) = raw_request(
        &fixture.app,
        "POST",
        "/api/v1/admin/assets",
        Some(&format!("multipart/form-data; boundary={boundary}")),
        multipart_body(boundary, "logo.png", Some("Brand logo")),
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let asset: Value = serde_json::from_slice(&bytes).unwrap();
    let asset_id = asset["id"].as_str().unwrap().to_owned();
    assert_eq!(asset["filename"], "logo.png");
    assert_eq!(asset["content_type"], "image/png");
    assert_eq!(asset["alt"], "Brand logo");

    let (status, list, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/assets?search=brand",
        None,
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["pagination"]["total"], 1);
    assert_eq!(list["data"][0]["id"], asset_id);

    let (status, picked, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/data-sources/asset?search=logo",
        None,
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{picked}");
    assert_eq!(picked["items"][0]["id"], asset_id);
    assert_eq!(picked["items"][0]["label"], "Brand logo");

    let (status, served, headers) = raw_request(
        &fixture.app,
        "GET",
        &format!("/api/v1/store/assets/{asset_id}"),
        None,
        Vec::new(),
        &[("host", HOST)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(served, PNG);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(
        headers["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );

    let (status, _, _) = request(
        &fixture.app,
        "DELETE",
        &format!("/api/v1/admin/assets/{asset_id}"),
        None,
        None,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = raw_request(
        &fixture.app,
        "GET",
        &format!("/api/v1/store/assets/{asset_id}"),
        None,
        Vec::new(),
        &[("host", HOST)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_rejects_unsupported_media() {
    let fixture = fixture().await;
    let admin_cookie = fixture.admin_cookie();
    let boundary = "fastcart-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"page.html\"\r\nContent-Type: text/html\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"<html><script>alert(1)</script></html>\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let (status, bytes, _) = raw_request(
        &fixture.app,
        "POST",
        "/api/v1/admin/assets",
        Some(&format!("multipart/form-data; boundary={boundary}")),
        body,
        &[
            ("cookie", admin_cookie.as_str()),
            ("x-csrf-token", fixture.csrf.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&bytes).contains("unsupported file type"));
}
