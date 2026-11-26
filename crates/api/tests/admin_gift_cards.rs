//! Route-level coverage for `/api/v1/admin/gift-cards`.
//!
//! The service layer, including redemption and its concurrency behaviour, is
//! covered in `crates/modules/payment/src/service/tests/gift_cards.rs`. These
//! tests drive the admin HTTP surface: issuing, listing, fetching and toggling
//! a card, plus the `payments.read` / `payments.write` permissions. Redemption
//! has no HTTP endpoint yet because nothing buys with a gift card; the admin
//! routes only manage cards.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_module_payment::PaymentService;
use fastcart_platform::{CreateUserInput, PlatformStore};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    context: Arc<AppContext>,
    owner_auth: String,
    csrf: String,
    store_a: StoreId,
    store_b: StoreId,
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
    let app = build_router(context.clone());

    let (status, _, cookies) = send(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(json!({
            "email": "owner@example.com",
            "password": "a-very-long-password"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "platform setup");
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let owner_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let store_a = create_store(
        &app,
        &owner_auth,
        &csrf,
        "Alpha",
        "alpha",
        "alpha.example.com",
    )
    .await;
    let store_b = create_store(&app, &owner_auth, &csrf, "Beta", "beta", "beta.example.com").await;

    Fixture {
        app,
        context,
        owner_auth,
        csrf,
        store_a,
        store_b,
        _dir: dir,
    }
}

async fn create_store(
    app: &Router,
    owner_auth: &str,
    csrf: &str,
    name: &str,
    slug: &str,
    domain: &str,
) -> StoreId {
    let (status, store, _) = send(
        app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": name,
            "slug": slug,
            "primary_domain": domain
        })),
        &[("cookie", owner_auth), ("x-csrf-token", csrf)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    StoreId::from_string(store["id"].as_str().unwrap())
}

async fn send(
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value, cookies)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

/// Signs up a user holding `roles` in `store` and returns their cookie pair
/// plus the CSRF token the pair carries.
async fn member_auth(
    fixture: &Fixture,
    email: &str,
    store: &StoreId,
    roles: &[&str],
) -> (String, String) {
    let member = fixture
        .context
        .auth
        .register(CreateUserInput::new(email), "member-password")
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, store, roles)
        .await
        .unwrap();

    let (status, _, cookies) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(json!({"email": email, "password": "member-password"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    (format!("fc_admin_session={session}; fc_csrf={csrf}"), csrf)
}

impl Fixture {
    /// Sends one request as the store owner.
    async fn admin(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
        store: &StoreId,
    ) -> (StatusCode, Value) {
        let (status, value, _) = send(
            &self.app,
            method,
            uri,
            body,
            &[
                ("cookie", self.owner_auth.as_str()),
                ("x-csrf-token", self.csrf.as_str()),
                ("x-store-id", store.as_str()),
            ],
        )
        .await;
        (status, value)
    }

    async fn issue(&self, store: &StoreId, body: Value) -> Value {
        let (status, card) = self
            .admin("POST", "/api/v1/admin/gift-cards", Some(body), store)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        card
    }
}

#[tokio::test]
async fn gift_card_lifecycle_over_http() {
    let fixture = fixture().await;

    let card = fixture
        .issue(
            &fixture.store_a,
            json!({"currency_code": "USD", "amount_minor": 5_000, "metadata": {"order": "order_1"}}),
        )
        .await;
    assert!(card["id"].as_str().unwrap().starts_with("gift_"));
    assert_eq!(card["currency_code"], "USD");
    assert_eq!(card["initial_amount_minor"], 5_000);
    assert_eq!(card["balance_minor"], 5_000);
    assert_eq!(card["status"], "active");
    // Codes are operationally needed to hand the card to its buyer, so admin
    // responses return the full code.
    let code = card["code"].as_str().unwrap();
    assert_eq!(code.chars().count(), 16, "{card}");
    assert_eq!(card["metadata"]["order"], "order_1");
    let card_id = card["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/gift-cards", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1);
    assert_eq!(page["data"][0]["id"], card_id.as_str());
    assert_eq!(page["data"][0]["code"], code);

    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/gift-cards/{card_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["code"], code);

    let (status, disabled) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/gift-cards/{card_id}/disable"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["status"], "disabled");

    // A disabled card rejects redemption at the service layer.
    let database = fixture
        .context
        .databases
        .database(&fixture.store_a)
        .await
        .unwrap();
    let payments = PaymentService::new(database);
    let rejected = payments.redeem_gift_card(code, 100).await.unwrap_err();
    assert!(
        matches!(rejected, fastcart_kernel::AppError::Conflict(_)),
        "{rejected}"
    );

    let (status, enabled) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/gift-cards/{card_id}/enable"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{enabled}");
    assert_eq!(enabled["status"], "active");

    let balance = payments.redeem_gift_card(code, 1_500).await.unwrap();
    assert_eq!(balance, 3_500);
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/gift-cards/{card_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["balance_minor"], 3_500);
}

#[tokio::test]
async fn gift_card_routes_validate_and_isolate_stores() {
    let fixture = fixture().await;

    // An explicit code is kept verbatim, and duplicates conflict.
    let card = fixture
        .issue(
            &fixture.store_a,
            json!({"currency_code": "EUR", "amount_minor": 1_000, "code": "GIFT-EUR-1"}),
        )
        .await;
    assert_eq!(card["code"], "GIFT-EUR-1");
    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/gift-cards",
            Some(json!({"currency_code": "EUR", "amount_minor": 1_000, "code": "GIFT-EUR-1"})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/gift-cards",
            Some(json!({"currency_code": "USD", "amount_minor": 0})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/gift-cards",
            Some(json!({"currency_code": "not-a-currency", "amount_minor": 100})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = fixture
        .admin(
            "GET",
            "/api/v1/admin/gift-cards/gift_missing",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // Issued cards live in one store's database only.
    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/gift-cards", None, &fixture.store_b)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 0, "{page}");
}

#[tokio::test]
async fn gift_card_routes_enforce_their_permissions() {
    let fixture = fixture().await;

    // `support-agent` holds none of the payment permissions.
    let (support, support_csrf) = member_auth(
        &fixture,
        "support@example.com",
        &fixture.store_a,
        &["support-agent"],
    )
    .await;
    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/gift-cards",
        None,
        &[
            ("cookie", support.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/gift-cards",
        Some(json!({"currency_code": "USD", "amount_minor": 100})),
        &[
            ("cookie", support.as_str()),
            ("x-csrf-token", support_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // `order-manager` holds both payment permissions.
    let (order_manager, order_manager_csrf) = member_auth(
        &fixture,
        "order-manager@example.com",
        &fixture.store_a,
        &["order-manager"],
    )
    .await;
    let (status, card, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/gift-cards",
        Some(json!({"currency_code": "USD", "amount_minor": 900})),
        &[
            ("cookie", order_manager.as_str()),
            ("x-csrf-token", order_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{card}");

    let card_id = card["id"].as_str().unwrap();
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/gift-cards/{card_id}/disable"),
        None,
        &[
            ("cookie", order_manager.as_str()),
            ("x-csrf-token", order_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
