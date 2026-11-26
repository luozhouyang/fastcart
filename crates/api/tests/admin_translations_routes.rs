//! Route-level coverage for platform store locales and admin translations.
//!
//! The services are covered elsewhere; these tests drive the HTTP surface:
//! authentication, permissions, validation and the single-default invariant.

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

impl Fixture {
    async fn owner(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, value, _) = send(
            &self.app,
            method,
            uri,
            body,
            &[
                ("cookie", self.owner_auth.as_str()),
                ("x-csrf-token", self.csrf.as_str()),
            ],
        )
        .await;
        (status, value)
    }

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
async fn store_locales_are_managed_over_http() {
    let fixture = fixture().await;
    let base = format!("/api/v1/platform/stores/{}/locales", fixture.store_a);

    let (status, body) = fixture.owner("GET", &base, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!([]));

    let (status, created) = fixture
        .owner(
            "POST",
            &base,
            Some(json!({"locale": "en_us", "label": "English"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["locale"], "en-US", "locales are normalized");
    assert_eq!(created["store_id"], fixture.store_a.as_str());
    assert_eq!(
        created["is_default"], true,
        "the first locale is the default"
    );
    assert_eq!(created["direction"], "ltr");

    let (status, added) = fixture
        .owner(
            "POST",
            &base,
            Some(json!({"locale": "fr-FR", "label": "Français", "direction": "ltr"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");
    assert_eq!(added["is_default"], false);

    let (status, duplicate) = fixture
        .owner("POST", &base, Some(json!({"locale": "fr_FR"})))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    assert_eq!(duplicate["error"]["code"], "CONFLICT");

    let (status, locales) = fixture.owner("GET", &base, None).await;
    assert_eq!(status, StatusCode::OK, "{locales}");
    assert_eq!(locales.as_array().unwrap().len(), 2);
    assert_eq!(locales[0]["locale"], "en-US");

    let (status, promoted) = fixture
        .owner("POST", &format!("{base}/fr-FR/default"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{promoted}");
    assert_eq!(promoted["is_default"], true);

    let (status, locales) = fixture.owner("GET", &base, None).await;
    assert_eq!(status, StatusCode::OK, "{locales}");
    let defaults: Vec<&Value> = locales
        .as_array()
        .unwrap()
        .iter()
        .filter(|locale| locale["is_default"].as_bool() == Some(true))
        .collect();
    assert_eq!(defaults.len(), 1, "exactly one default remains: {locales}");
    assert_eq!(defaults[0]["locale"], "fr-FR");

    let (status, updated) = fixture
        .owner(
            "PATCH",
            &format!("{base}/fr-FR"),
            Some(json!({"label": "Français (FR)", "direction": "rtl"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["label"], "Français (FR)");
    assert_eq!(updated["direction"], "rtl");

    let (status, error) = fixture
        .owner(
            "PATCH",
            &format!("{base}/fr-FR"),
            Some(json!({"direction": "sideways"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    let (status, _) = fixture
        .owner("DELETE", &format!("{base}/en-US"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, error) = fixture
        .owner("DELETE", &format!("{base}/fr-FR"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["error"]["code"], "CONFLICT");

    let (status, missing) = fixture
        .owner("POST", &format!("{base}/de-DE/default"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");

    let (status, invalid) = fixture
        .owner("POST", &base, Some(json!({"locale": "not a locale"})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
}

#[tokio::test]
async fn store_locales_are_isolated_and_authenticated() {
    let fixture = fixture().await;

    let base_a = format!("/api/v1/platform/stores/{}/locales", fixture.store_a);
    let base_b = format!("/api/v1/platform/stores/{}/locales", fixture.store_b);
    let (status, _) = fixture
        .owner("POST", &base_a, Some(json!({"locale": "fr-FR"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, locales) = fixture.owner("GET", &base_b, None).await;
    assert_eq!(status, StatusCode::OK, "{locales}");
    assert_eq!(locales, json!([]));

    let (status, error) = fixture
        .owner("POST", &format!("{base_b}/fr-FR/default"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    let (status, _, _) = send(&fixture.app, "GET", &base_a, None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn store_locale_writes_require_settings_write() {
    let fixture = fixture().await;
    let base = format!("/api/v1/platform/stores/{}/locales", fixture.store_a);

    let member = fixture
        .context
        .auth
        .register(
            CreateUserInput::new("support@example.com"),
            "member-password",
        )
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, &fixture.store_a, &["support-agent"])
        .await
        .unwrap();
    let (status, _, cookies) = send(
        &fixture.app,
        "POST",
        "/api/v1/platform/auth/login",
        Some(json!({"email": "support@example.com", "password": "member-password"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();

    let (status, locales, _) = send(
        &fixture.app,
        "GET",
        &base,
        None,
        &[("cookie", &format!("fc_admin_session={session}"))],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{locales}");

    let (status, error, _) = send(
        &fixture.app,
        "POST",
        &base,
        Some(json!({"locale": "fr-FR"})),
        &[
            (
                "cookie",
                &format!("fc_admin_session={session}; fc_csrf={csrf}"),
            ),
            ("x-csrf-token", &csrf),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
}

#[tokio::test]
async fn translations_are_upserted_listed_and_deleted_over_http() {
    let fixture = fixture().await;
    let uri = "/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1";

    let (status, empty) = fixture.admin("GET", uri, None, &fixture.store_a).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty, json!([]));

    let (status, created) = fixture
        .admin(
            "PUT",
            &format!("{uri}&locale=fr_fr"),
            Some(json!({"fields": {"title": "Chaise", "description": "Bois"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["locale"], "fr-FR", "locales are normalized");
    assert_eq!(created["resource_type"], "product");
    assert_eq!(created["fields"]["title"], "Chaise");

    let (status, listed) = fixture.admin("GET", uri, None, &fixture.store_a).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["locale"], "fr-FR");

    let (status, scoped) = fixture
        .admin(
            "GET",
            &format!("{uri}&locale=fr-FR"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{scoped}");
    assert_eq!(scoped[0]["fields"]["title"], "Chaise");

    // A second upsert replaces the row instead of creating another one.
    let (status, replaced) = fixture
        .admin(
            "PUT",
            &format!("{uri}&locale=fr-FR"),
            Some(json!({"fields": {"title": "Chaise longue"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert_eq!(replaced["id"], created["id"]);
    assert_eq!(replaced["fields"]["title"], "Chaise longue");

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("{uri}&locale=fr-FR"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, listed) = fixture.admin("GET", uri, None, &fixture.store_a).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn translation_routes_validate_their_input() {
    let fixture = fixture().await;
    let store = &fixture.store_a;

    // Missing locale on a write.
    let (status, error) = fixture
        .admin(
            "PUT",
            "/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1",
            Some(json!({"fields": {"title": "Chaise"}})),
            store,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // Unknown resource type.
    let (status, error) = fixture
        .admin(
            "PUT",
            "/api/v1/admin/content/translations?resource_type=order&resource_id=ord_1&locale=fr-FR",
            Some(json!({"fields": {"title": "Chaise"}})),
            store,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    // Fields must be an object.
    let (status, error) = fixture
        .admin(
            "PUT",
            "/api/v1/admin/content/translations?resource_type=page&resource_id=page_1&locale=fr-FR",
            Some(json!({"fields": ["not", "an", "object"]})),
            store,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // Empty resource id.
    let (status, error) = fixture
        .admin(
            "PUT",
            "/api/v1/admin/content/translations?resource_type=page&resource_id=%20&locale=fr-FR",
            Some(json!({"fields": {"title": "Accueil"}})),
            store,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // Deleting a missing row.
    let (status, error) = fixture
        .admin(
            "DELETE",
            "/api/v1/admin/content/translations?resource_type=page&resource_id=page_1&locale=fr-FR",
            None,
            store,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    assert_eq!(
        fixture
            .admin(
                "GET",
                "/api/v1/admin/content/translations?resource_type=page&resource_id=page_1",
                None,
                store,
            )
            .await
            .1,
        json!([])
    );
}

#[tokio::test]
async fn translations_are_isolated_per_store() {
    let fixture = fixture().await;
    let uri = "/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1";

    let (status, _) = fixture
        .admin(
            "PUT",
            &format!("{uri}&locale=fr-FR"),
            Some(json!({"fields": {"title": "Chaise"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, listed) = fixture.admin("GET", uri, None, &fixture.store_b).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed, json!([]));

    let (status, _) = fixture
        .admin(
            "DELETE",
            &format!("{uri}&locale=fr-FR"),
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, listed) = fixture.admin("GET", uri, None, &fixture.store_a).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn translation_permissions_follow_settings() {
    let fixture = fixture().await;
    let uri = "/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1";

    // `content-editor` holds settings.read but not settings.write.
    let member = fixture
        .context
        .auth
        .register(
            CreateUserInput::new("editor@example.com"),
            "member-password",
        )
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, &fixture.store_a, &["content-editor"])
        .await
        .unwrap();
    let (status, _, cookies) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/auth/login",
        Some(json!({"email": "editor@example.com", "password": "member-password"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let headers = [
        (
            "cookie",
            format!("fc_admin_session={session}; fc_csrf={csrf}"),
        ),
        ("x-csrf-token", csrf.clone()),
        ("x-store-id", fixture.store_a.to_string()),
    ];
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();

    let (status, listed, _) = send(&fixture.app, "GET", uri, None, &headers).await;
    assert_eq!(status, StatusCode::OK, "{listed}");

    let (status, error, _) = send(
        &fixture.app,
        "PUT",
        &format!("{uri}&locale=fr-FR"),
        Some(json!({"fields": {"title": "Chaise"}})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");

    let (status, _, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/content/translations?resource_type=product&resource_id=prd_1",
        None,
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
