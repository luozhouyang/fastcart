//! Route-level coverage for `/api/v1/admin/campaigns`.
//!
//! The service layer is covered elsewhere; these tests drive the real HTTP
//! surface: authentication, the `promotions.read` / `promotions.write`
//! permissions, per-store isolation, request validation and response shaping.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
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
    assert_eq!(status, StatusCode::OK);
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

    async fn create_campaign(&self, store: &StoreId, name: &str) -> Value {
        let (status, campaign) = self
            .admin(
                "POST",
                "/api/v1/admin/campaigns",
                Some(json!({"name": name})),
                store,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{campaign}");
        campaign
    }
}

#[tokio::test]
async fn campaign_lifecycle_over_http() {
    let fixture = fixture().await;

    let (status, campaign) = fixture
        .admin(
            "POST",
            "/api/v1/admin/campaigns",
            Some(json!({
                "name": "Summer",
                "description": "Summer sale",
                "budget_type": "usage",
                "budget_limit": 50
            })),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{campaign}");
    assert!(
        campaign["id"].as_str().unwrap().starts_with("camp_"),
        "{campaign}"
    );
    assert_eq!(campaign["name"], "Summer");
    assert_eq!(campaign["budget_type"], "usage");
    assert_eq!(campaign["budget_limit"], 50);
    assert_eq!(campaign["budget_used"], 0);
    let campaign_id = campaign["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/campaigns", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1);
    assert_eq!(page["data"][0]["id"], campaign_id.as_str());

    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["description"], "Summer sale");

    let (status, updated) = fixture
        .admin(
            "PATCH",
            &format!("/api/v1/admin/campaigns/{campaign_id}"),
            Some(json!({"name": "Summer Sale", "budget_limit": 25})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "Summer Sale");
    assert_eq!(updated["budget_limit"], 25);

    // A promotion has to exist before it can be linked.
    let (status, promotion) = fixture
        .admin(
            "POST",
            "/api/v1/admin/promotions",
            Some(json!({
                "title": "Summer 10%",
                "promotion_type": "percentage",
                "value": 1_000
            })),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{promotion}");
    let promotion_id = promotion["id"].as_str().unwrap().to_owned();

    let (status, _) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            Some(json!({"promotion_id": promotion_id})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, linked) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{linked}");
    assert_eq!(linked, json!([promotion_id]));

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/campaigns/{campaign_id}/promotions/{promotion_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, linked) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(linked, json!([]));

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, missing) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn campaign_routes_require_authentication() {
    let fixture = fixture().await;

    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/campaigns",
        None,
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/campaigns",
        Some(json!({"name": "Sneaky"})),
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Selecting a store the owner does not belong to is rejected before the
    // store database opens.
    let (status, body) = fixture
        .admin("GET", "/api/v1/admin/campaigns", None, &StoreId::new())
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn campaign_routes_enforce_their_permissions() {
    let fixture = fixture().await;

    // `order-manager` holds neither campaign permission.
    let (order_manager, order_manager_csrf) = member_auth(
        &fixture,
        "order-manager@example.com",
        &fixture.store_a,
        &["order-manager"],
    )
    .await;
    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/campaigns",
        None,
        &[
            ("cookie", order_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/campaigns",
        Some(json!({"name": "Not mine"})),
        &[
            ("cookie", order_manager.as_str()),
            ("x-csrf-token", order_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // `product-manager` may read and write promotions.
    let (product_manager, product_manager_csrf) = member_auth(
        &fixture,
        "product-manager@example.com",
        &fixture.store_a,
        &["product-manager"],
    )
    .await;
    let (status, page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/campaigns",
        None,
        &[
            ("cookie", product_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    let (status, created, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/campaigns",
        Some(json!({"name": "Product manager campaign"})),
        &[
            ("cookie", product_manager.as_str()),
            ("x-csrf-token", product_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
}

#[tokio::test]
async fn campaigns_are_isolated_per_store() {
    let fixture = fixture().await;
    let campaign = fixture
        .create_campaign(&fixture.store_a, "Alpha campaign")
        .await;
    let campaign_id = campaign["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/campaigns", None, &fixture.store_b)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 0);

    for (method, uri, body) in [
        (
            "GET",
            format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
        ),
        (
            "PATCH",
            format!("/api/v1/admin/campaigns/{campaign_id}"),
            Some(json!({"name": "Hijacked"})),
        ),
        (
            "DELETE",
            format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
        ),
        (
            "GET",
            format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            Some(json!({"promotion_id": "promo_other"})),
        ),
    ] {
        let (status, body) = fixture.admin(method, &uri, body, &fixture.store_b).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {body}");
    }

    // Store A's campaign is untouched.
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/campaigns/{campaign_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["name"], "Alpha campaign");
}

#[tokio::test]
async fn campaign_creation_validates_its_input() {
    let fixture = fixture().await;

    for body in [
        json!({"name": "   "}),
        json!({"name": "x", "budget_type": "spend", "budget_limit": 100}),
        json!({"name": "x", "budget_limit": 0}),
        json!({"description": "no name"}),
    ] {
        let (status, error) = fixture
            .admin(
                "POST",
                "/api/v1/admin/campaigns",
                Some(body.clone()),
                &fixture.store_a,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR", "{body}");
    }

    // Nothing was persisted by the rejected requests.
    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/campaigns", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["pagination"]["total"], 0);

    // Linking an unknown promotion is a not-found, not a silent success.
    let campaign = fixture
        .create_campaign(&fixture.store_a, "Kitchen sink")
        .await;
    let campaign_id = campaign["id"].as_str().unwrap().to_owned();
    let (status, error) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/campaigns/{campaign_id}/promotions"),
            Some(json!({"promotion_id": "promo_ghost"})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
}
