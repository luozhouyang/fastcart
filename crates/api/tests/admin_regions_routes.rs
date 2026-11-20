//! Route-level coverage for `/api/v1/admin/regions` and
//! `/api/v1/admin/publishable-api-keys`.
//!
//! The services are covered elsewhere; these tests drive the real HTTP surface:
//! authentication, the `settings.read` / `settings.write` permissions,
//! per-store isolation, request validation and the one-time key secret.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{StoreId, now_timestamp};
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

/// Inserts a sales channel so publishable keys have a scope to point at.
async fn seed_sales_channel(fixture: &Fixture, store: &StoreId, id: &str) {
    let db = fixture.context.databases.database(store).await.unwrap();
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO sales_channels (id, name, is_disabled, metadata, created_at, updated_at) \
         VALUES (?, ?, 0, '{}', ?, ?)",
    )
    .bind(id)
    .bind(format!("Channel {id}"))
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
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
}

#[tokio::test]
async fn region_and_key_lifecycle_over_http() {
    let fixture = fixture().await;

    let (status, region) = fixture
        .admin(
            "POST",
            "/api/v1/admin/regions",
            Some(json!({
                "name": "Europe",
                "currency_code": "eur",
                "countries": ["fr", "de"],
                "payment_providers": ["manual"]
            })),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{region}");
    assert!(
        region["id"].as_str().unwrap().starts_with("reg_"),
        "{region}"
    );
    assert_eq!(region["currency_code"], "EUR");
    assert_eq!(region["countries"], json!(["DE", "FR"]));
    assert_eq!(region["payment_providers"], json!(["manual"]));
    assert_eq!(region["is_default"], false);
    let region_id = region["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/regions", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1);
    assert_eq!(page["data"][0]["id"], region_id.as_str());

    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["name"], "Europe");

    let (status, updated) = fixture
        .admin(
            "PATCH",
            &format!("/api/v1/admin/regions/{region_id}"),
            Some(json!({"name": "European Union", "is_default": true})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "European Union");
    assert_eq!(updated["is_default"], true);

    // Publishable keys: the secret is returned exactly once.
    seed_sales_channel(&fixture, &fixture.store_a, "chan_web").await;
    let (status, created) = fixture
        .admin(
            "POST",
            "/api/v1/admin/publishable-api-keys",
            Some(json!({"title": "Web channel", "sales_channel_ids": ["chan_web"]})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let token = created["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("pk_"), "{created}");
    assert_eq!(created["title"], "Web channel");
    assert_eq!(created["sales_channel_ids"], json!(["chan_web"]));
    assert_eq!(
        created["token_prefix"].as_str().unwrap().len(),
        12,
        "{created}"
    );
    assert!(created.get("token_hash").is_none(), "{created}");
    assert!(created["revoked_at"].is_null(), "{created}");
    let key_id = created["id"].as_str().unwrap().to_owned();
    assert!(key_id.starts_with("pk_"), "{created}");

    let (status, keys) = fixture
        .admin(
            "GET",
            "/api/v1/admin/publishable-api-keys",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{keys}");
    assert_eq!(keys.as_array().unwrap().len(), 1);
    assert_eq!(keys[0]["id"], key_id.as_str());
    assert!(
        keys[0].get("token").is_none(),
        "the list must never re-expose the secret: {keys}"
    );

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/publishable-api-keys/{key_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, keys) = fixture
        .admin(
            "GET",
            "/api/v1/admin/publishable-api-keys",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(keys[0]["revoked_at"].is_i64(), "{keys}");

    // The default region is protected until the flag is cleared.
    let (status, conflict) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "CONFLICT");

    let (status, cleared) = fixture
        .admin(
            "PATCH",
            &format!("/api/v1/admin/regions/{region_id}"),
            Some(json!({"is_default": false})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["is_default"], false);

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, missing) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn region_routes_require_authentication() {
    let fixture = fixture().await;

    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/regions",
        None,
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"]["code"], "UNAUTHORIZED");

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/publishable-api-keys",
        Some(json!({"title": "Sneaky", "sales_channel_ids": ["chan_web"]})),
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A cookie-authenticated write without the CSRF pair is rejected.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/regions",
        Some(json!({"name": "No CSRF", "currency_code": "USD", "countries": ["US"]})),
        &[
            ("cookie", fixture.owner_auth.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn region_routes_enforce_their_permissions() {
    let fixture = fixture().await;

    // `product-manager` may read settings but not write them.
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
        "/api/v1/admin/regions",
        None,
        &[
            ("cookie", product_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");

    let (status, body, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/regions",
        Some(json!({"name": "Not mine", "currency_code": "USD", "countries": ["US"]})),
        &[
            ("cookie", product_manager.as_str()),
            ("x-csrf-token", product_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    let (status, _, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/publishable-api-keys",
        Some(json!({"title": "Not mine", "sales_channel_ids": ["chan_web"]})),
        &[
            ("cookie", product_manager.as_str()),
            ("x-csrf-token", product_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // `order-manager` holds neither settings permission.
    let (order_manager, _) = member_auth(
        &fixture,
        "order-manager@example.com",
        &fixture.store_a,
        &["order-manager"],
    )
    .await;
    let (status, _, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/regions",
        None,
        &[
            ("cookie", order_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/publishable-api-keys",
        None,
        &[
            ("cookie", order_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn regions_and_keys_are_isolated_per_store() {
    let fixture = fixture().await;

    let (status, region) = fixture
        .admin(
            "POST",
            "/api/v1/admin/regions",
            Some(json!({"name": "Alpha region", "currency_code": "USD", "countries": ["US"]})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{region}");
    let region_id = region["id"].as_str().unwrap().to_owned();

    seed_sales_channel(&fixture, &fixture.store_a, "chan_alpha").await;
    let (status, key) = fixture
        .admin(
            "POST",
            "/api/v1/admin/publishable-api-keys",
            Some(json!({"title": "Alpha key", "sales_channel_ids": ["chan_alpha"]})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{key}");
    let key_id = key["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/regions", None, &fixture.store_b)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 0);

    let (status, body) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, _) = fixture
        .admin(
            "PATCH",
            &format!("/api/v1/admin/regions/{region_id}"),
            Some(json!({"name": "Hijacked"})),
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, keys) = fixture
        .admin(
            "GET",
            "/api/v1/admin/publishable-api-keys",
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{keys}");
    assert_eq!(keys, json!([]));

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("/api/v1/admin/publishable-api-keys/{key_id}"),
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Store A's region and key survived every cross-store attempt.
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/regions/{region_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["name"], "Alpha region");
    let (status, keys) = fixture
        .admin(
            "GET",
            "/api/v1/admin/publishable-api-keys",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys[0]["id"], key_id.as_str());
    assert!(keys[0]["revoked_at"].is_null(), "{keys}");
}

#[tokio::test]
async fn region_and_key_creation_validates_its_input() {
    let fixture = fixture().await;

    for body in [
        json!({"name": "Nowhere", "currency_code": "USD", "countries": []}),
        json!({"name": "Nowhere", "currency_code": "NOT_A_CURRENCY", "countries": ["US"]}),
        json!({"name": "Nowhere", "currency_code": "USD", "countries": ["ZZ"]}),
        json!({"name": "  ", "currency_code": "USD", "countries": ["US"]}),
        json!({"currency_code": "USD", "countries": ["US"]}),
    ] {
        let (status, error) = fixture
            .admin(
                "POST",
                "/api/v1/admin/regions",
                Some(body.clone()),
                &fixture.store_a,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {error}");
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR", "{body}");
    }

    // An empty channel list is invalid; an unknown channel is not found.
    let (status, error) = fixture
        .admin(
            "POST",
            "/api/v1/admin/publishable-api-keys",
            Some(json!({"title": "Key", "sales_channel_ids": []})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    let (status, error) = fixture
        .admin(
            "POST",
            "/api/v1/admin/publishable-api-keys",
            Some(json!({"title": "Key", "sales_channel_ids": ["chan_ghost"]})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["error"]["code"], "NOT_FOUND");

    // Nothing was persisted by the rejected requests.
    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/regions", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["pagination"]["total"], 0);
    let (status, keys) = fixture
        .admin(
            "GET",
            "/api/v1/admin/publishable-api-keys",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys, json!([]));
}
