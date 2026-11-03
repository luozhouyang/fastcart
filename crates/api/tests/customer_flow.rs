//! Storefront customer authentication and profile flow over HTTP.

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
    admin_auth: String,
    csrf: String,
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
    let app = build_router(context);

    // Bootstrap admin and a store.
    let (status, _, cookies) = send(
        &app,
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
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
    let (status, _, _) = send(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": "shop.example.com"
        })),
        &[("cookie", auth.as_str()), ("x-csrf-token", csrf.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Keep the temp directory alive for the process lifetime.
    std::mem::forget(dir);
    Fixture {
        app,
        admin_auth: auth,
        csrf,
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value, Vec<String>) {
    send_to_host(app, "shop.example.com", method, uri, body, headers).await
}

async fn send_to_host(
    app: &Router,
    host: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value, Vec<String>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", host);
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
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value, cookies)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

#[tokio::test]
async fn customers_register_login_and_manage_their_own_profile() {
    let fixture = fixture().await;

    let (status, body, cookies) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "hunter2hunter2",
            "first_name": "Ada",
            "last_name": "Lovelace"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["customer"]["email"], "buyer@example.com");
    let session = cookie_value(&cookies, "fc_customer_session").expect("customer session");
    let csrf = cookie_value(&cookies, "fc_customer_csrf").expect("customer csrf");
    let auth = format!("fc_customer_session={session}; fc_customer_csrf={csrf}");

    let (status, me, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["email"], "buyer@example.com");

    // Cookie-auth mutation without CSRF is rejected.
    let (status, _, _) = send(
        &fixture.app,
        "PATCH",
        "/api/v1/store/customers/me",
        Some(serde_json::json!({"first_name": "Grace"})),
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let headers = [("cookie", auth.as_str()), ("x-csrf-token", csrf.as_str())];
    let (status, updated, _) = send(
        &fixture.app,
        "PATCH",
        "/api/v1/store/customers/me",
        Some(serde_json::json!({"first_name": "Grace", "phone": "+1 555 0100"})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["first_name"], "Grace");

    let (status, address, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/me/addresses",
        Some(serde_json::json!({
            "first_name": "Grace",
            "last_name": "Hopper",
            "address_1": "1 Navy Way",
            "city": "Arlington",
            "postal_code": "22201",
            "country_code": "US"
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{address}");
    assert_eq!(address["country_code"], "US");

    let (status, addresses, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me/addresses",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(addresses.as_array().unwrap().len(), 1);

    // Logout requires CSRF and revokes the session.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/logout",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Login rejects bad credentials and accepts good ones.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/login",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "wrong"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body, cookies) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/login",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(cookie_value(&cookies, "fc_customer_session").is_some());

    // Customer identities and sessions are scoped to one store database.
    let admin_headers = [
        ("cookie", fixture.admin_auth.as_str()),
        ("x-csrf-token", fixture.csrf.as_str()),
    ];
    let (status, second_store, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Second",
            "slug": "second",
            "primary_domain": "second.example.com"
        })),
        &admin_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second_store}");

    let (status, _, _) = send_to_host(
        &fixture.app,
        "second.example.com",
        "GET",
        "/api/v1/store/customers/me",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, second_customer, _) = send_to_host(
        &fixture.app,
        "second.example.com",
        "POST",
        "/api/v1/store/customers",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_customer}");
    assert_eq!(second_customer["customer"]["email"], "buyer@example.com");
}
