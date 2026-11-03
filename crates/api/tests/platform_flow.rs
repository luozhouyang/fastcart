//! End-to-end platform HTTP flow tests.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Fixture {
    app: Router,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let platform = PlatformStore::in_memory().await.unwrap();
    let databases = Arc::new(StoreDatabaseManager::new(
        dir.path().join("stores"),
        DatabaseOptions::default().with_max_connections(2),
    ));
    let config = ApiConfig {
        cookie_secure: false,
        backup_dir: dir.path().join("backups"),
        ..ApiConfig::default()
    };
    let context = Arc::new(AppContext::new(
        config,
        platform,
        databases,
        StoreCache::default_capacity(),
    ));
    Fixture {
        app: build_router(context),
        _dir: dir,
    }
}

async fn json_request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value, Vec<String>) {
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
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, body, cookies)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        if cookie_name.trim() == name {
            Some(rest.split(';').next()?.to_owned())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn health_endpoints_report_ok() {
    let fixture = fixture().await;
    let (status, body, _) = json_request(&fixture.app, "GET", "/health", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    let (status, body, _) = json_request(&fixture.app, "GET", "/health/ready", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn setup_creates_the_first_user_and_session_once() {
    let fixture = fixture().await;
    let (status, body, cookies) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2",
            "first_name": "Ada",
            "last_name": "Lovelace"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], "owner@example.com");
    assert!(
        body["csrf_token"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    let session = cookie_value(&cookies, "fc_admin_session").expect("session cookie");
    let csrf = cookie_value(&cookies, "fc_csrf").expect("csrf cookie");

    let (status, body, _) = json_request(
        &fixture.app,
        "GET",
        "/api/v1/platform/auth/me",
        None,
        &[("cookie", &format!("fc_admin_session={session}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], "owner@example.com");
    assert!(body["csrf_token"].is_null());

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "second@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Cookie-authenticated mutation without the CSRF header is rejected.
    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/logout",
        None,
        &[(
            "cookie",
            &format!("fc_admin_session={session}; fc_csrf={csrf}"),
        )],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/logout",
        None,
        &[
            (
                "cookie",
                &format!("fc_admin_session={session}; fc_csrf={csrf}"),
            ),
            ("x-csrf-token", &csrf),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = json_request(
        &fixture.app,
        "GET",
        "/api/v1/platform/auth/me",
        None,
        &[("cookie", &format!("fc_admin_session={session}"))],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_rejects_bad_credentials_and_unauthenticated_requests() {
    let fixture = fixture().await;
    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/login",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "wrong-password"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) =
        json_request(&fixture.app, "GET", "/api/v1/platform/stores", None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn store_lifecycle_and_domain_management() {
    let fixture = fixture().await;
    let (_, _, cookies) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
    let headers = [("cookie", auth.as_str()), ("x-csrf-token", csrf.as_str())];

    let (status, store, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo Store",
            "slug": "demo",
            "primary_domain": "demo.example.com",
            "currency_code": "USD"
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    assert_eq!(store["status"], "active");
    let store_id = store["id"].as_str().unwrap().to_owned();

    let (status, stores, _) = json_request(
        &fixture.app,
        "GET",
        "/api/v1/platform/stores",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stores["pagination"]["total"], 1);
    assert_eq!(stores["data"][0]["id"], store_id);

    let (status, domain, _) = json_request(
        &fixture.app,
        "POST",
        &format!("/api/v1/platform/stores/{store_id}/domains"),
        Some(serde_json::json!({"hostname": "www.example.com", "is_primary": false})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{domain}");
    assert_eq!(domain["hostname"], "www.example.com");

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Duplicate",
            "slug": "duplicate",
            "primary_domain": "demo.example.com"
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, _) = json_request(
        &fixture.app,
        "PATCH",
        &format!("/api/v1/platform/stores/{store_id}"),
        Some(serde_json::json!({"name": "Renamed"})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        &format!("/api/v1/platform/stores/{store_id}/suspend"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        &format!("/api/v1/platform/stores/{store_id}/resume"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        &format!("/api/v1/platform/stores/{store_id}/delete"),
        Some(serde_json::json!({"backup": true, "purge_data": true})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn storefront_requests_resolve_the_host_trustworthily() {
    let fixture = fixture().await;
    let (_, _, cookies) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(serde_json::json!({
            "email": "owner@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
    let headers = [("cookie", auth.as_str()), ("x-csrf-token", csrf.as_str())];

    let (status, _, _) = json_request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": "demo.example.com"
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Store resolution must pass before the commerce route runs.
    let (status, body, _) = json_request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "demo.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body, _) = json_request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "unknown.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
