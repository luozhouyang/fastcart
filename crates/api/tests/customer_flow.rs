//! Storefront customer authentication and profile flow over HTTP.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_email::{SEND_EMAIL_JOB_TYPE, SendEmailJob};
use fastcart_infrastructure::{DatabaseOptions, JobQueue, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Fixture {
    app: Router,
    admin_auth: String,
    csrf: String,
    databases: Arc<StoreDatabaseManager>,
    store_id: StoreId,
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
        databases.clone(),
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
    let (status, store, _) = send(
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
    let store_id = StoreId::from_string(store["id"].as_str().unwrap());

    Fixture {
        app,
        admin_auth: auth,
        csrf,
        databases,
        store_id,
        _dir: dir,
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

async fn claim_email_job(fixture: &Fixture) -> SendEmailJob {
    let database = fixture.databases.database(&fixture.store_id).await.unwrap();
    let queue = JobQueue::new(database.pool().clone());
    let jobs = queue.claim("test-worker", 10).await.unwrap();
    let record = jobs
        .into_iter()
        .find(|job| job.job_type == SEND_EMAIL_JOB_TYPE)
        .expect("an enqueued email job");
    serde_json::from_str(&record.payload).expect("a valid email job payload")
}

fn token_from_body(body: &str) -> String {
    body.rsplit_once(':')
        .map(|(_, token)| token.trim().to_owned())
        .expect("an email body ending in a token")
}

#[tokio::test]
async fn customers_verify_email_and_reset_password_over_http() {
    let fixture = fixture().await;

    let (status, body, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Existing accounts receive the generic acknowledgement.
    let (status, accepted, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/request",
        Some(serde_json::json!({"email": "BUYER@Example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    assert_eq!(accepted, serde_json::json!({"accepted": true}));

    // Unknown accounts receive the exact same response and no email.
    let (status, unknown, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/request",
        Some(serde_json::json!({"email": "nobody@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{unknown}");
    assert_eq!(unknown, accepted);

    let job = claim_email_job(&fixture).await;
    assert_eq!(job.to, "buyer@example.com");
    let verification_token = token_from_body(job.text.as_deref().unwrap());

    let (status, verified, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/confirm",
        Some(serde_json::json!({"token": verification_token})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{verified}");
    assert_eq!(verified["email_verified"], true);

    // A token can only be consumed once.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/confirm",
        Some(serde_json::json!({"token": verification_token})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Already verified accounts still get the generic response and no email.
    let (status, already_verified, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/request",
        Some(serde_json::json!({"email": "buyer@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(already_verified, accepted);
    let database = fixture.databases.database(&fixture.store_id).await.unwrap();
    let queue = JobQueue::new(database.pool().clone());
    assert!(queue.claim("test-worker-2", 10).await.unwrap().is_empty());

    // Reset requests are generic as well.
    let (status, reset_accepted, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/password-reset/request",
        Some(serde_json::json!({"email": "BUYER@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{reset_accepted}");
    assert_eq!(reset_accepted, accepted);

    let job = claim_email_job(&fixture).await;
    assert_eq!(job.to, "buyer@example.com");
    let reset_token = token_from_body(job.text.as_deref().unwrap());

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/password-reset/confirm",
        Some(serde_json::json!({
            "token": reset_token,
            "password": "new-password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The old password no longer works and the new one does.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/login",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, logged_in, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/login",
        Some(serde_json::json!({
            "email": "buyer@example.com",
            "password": "new-password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{logged_in}");
    assert_eq!(logged_in["customer"]["email"], "buyer@example.com");

    // Reset tokens are single use as well.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/password-reset/confirm",
        Some(serde_json::json!({
            "token": reset_token,
            "password": "another-password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Unknown emails never produce a reset email.
    let (status, unknown_reset, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/password-reset/request",
        Some(serde_json::json!({"email": "nobody@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(unknown_reset, accepted);
    assert!(queue.claim("test-worker-3", 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn auth_email_requests_are_rate_limited_per_email() {
    let fixture = fixture().await;

    for _ in 0..5 {
        let (status, body, _) = send(
            &fixture.app,
            "POST",
            "/api/v1/store/customers/email-verification/request",
            Some(serde_json::json!({"email": "nobody@example.com"})),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    }

    let (status, body, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/email-verification/request",
        Some(serde_json::json!({"email": "nobody@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    // The reset flow uses its own key for the same address.
    let (status, body, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/store/customers/password-reset/request",
        Some(serde_json::json!({"email": "nobody@example.com"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
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
    assert_eq!(me["customer"]["email"], "buyer@example.com");

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
    assert_eq!(updated["customer"]["first_name"], "Grace");

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
    assert_eq!(address["address"]["country_code"], "US");

    let (status, addresses, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/store/customers/me/addresses",
        None,
        &[("cookie", auth.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(addresses["addresses"].as_array().unwrap().len(), 1);

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
