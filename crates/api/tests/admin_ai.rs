//! Route-level coverage for `/api/v1/admin/ai`.
//!
//! These tests drive the real HTTP surface: authentication, the
//! `settings.read` / `settings.write` permissions, the tool catalog, task
//! creation and the approval-gated state machine. No model is called and no
//! tool is executed: a task only records its invocation and moves through the
//! transitions defined by `fastcart-ai`.

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
}

#[tokio::test]
async fn tool_catalog_lists_the_stable_builtin_tools() {
    let fixture = fixture().await;

    let (status, tools) = fixture
        .admin("GET", "/api/v1/admin/ai/tools", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{tools}");

    let tools = tools.as_array().expect("tool list");
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "block.list_types",
            "order.get",
            "page.get",
            "page.publish",
            "page.update",
        ]
    );

    for tool in tools {
        assert!(
            tool["input_schema"].is_object(),
            "every tool carries a JSON Schema object: {tool}"
        );
        let mutates = tool["mutates"].as_bool().unwrap();
        assert_eq!(
            tool["requires_approval"].as_bool().unwrap(),
            mutates,
            "mutating tools require approval and read-only tools do not: {tool}"
        );
    }
    let publish = tools
        .iter()
        .find(|tool| tool["name"] == "page.publish")
        .unwrap();
    assert_eq!(publish["mutates"], true);
    assert_eq!(publish["requires_approval"], true);
}

#[tokio::test]
async fn task_lifecycle_is_recorded_and_approval_gated() {
    let fixture = fixture().await;

    let (status, created) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({
                "tool": "page.update",
                "input": {"id": "page_1", "title": "Draft"}
            })),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(created["id"].as_str().unwrap().starts_with("task_"));
    assert_eq!(created["tool"], "page.update");
    assert_eq!(created["status"], "pending");
    assert_eq!(created["input"], json!({"id": "page_1", "title": "Draft"}));
    assert!(created["created_by"].is_string(), "{created}");
    let task_id = created["id"].as_str().unwrap().to_owned();

    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["status"], "pending");

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/ai/tasks", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1);
    assert_eq!(page["data"][0]["id"], task_id.as_str());

    // Approval resumes a pending task.
    let (status, approved) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{task_id}/approve"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "running");

    // Rejection cancels a running task; the terminal state is final.
    let (status, rejected) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{task_id}/reject"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["status"], "cancelled");

    let (status, conflict) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{task_id}/approve"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "CONFLICT");
    assert_eq!(conflict["error"]["details"]["from"], "cancelled");
    assert_eq!(conflict["error"]["details"]["to"], "running");

    // The rejected task is still recorded with its final status.
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["status"], "cancelled");
}

#[tokio::test]
async fn rejection_cancels_a_pending_task() {
    let fixture = fixture().await;

    let (status, created) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({"tool": "page.publish", "input": {"id": "page_1"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let task_id = created["id"].as_str().unwrap().to_owned();

    let (status, rejected) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{task_id}/reject"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["status"], "cancelled");
}

#[tokio::test]
async fn unknown_tools_and_invalid_input_are_rejected() {
    let fixture = fixture().await;

    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({"tool": "page.delete", "input": {"id": "page_1"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown AI tool"),
        "{body}"
    );

    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({"tool": "page.get", "input": ["not", "an", "object"]})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // A missing task is a 404, not a silent success.
    let (status, body) = fixture
        .admin(
            "GET",
            "/api/v1/admin/ai/tasks/task_missing",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks/task_missing/approve",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn ai_routes_enforce_their_permissions() {
    let fixture = fixture().await;

    // `product-manager` holds settings.read but not settings.write.
    let (product_manager, product_manager_csrf) = member_auth(
        &fixture,
        "product-manager@example.com",
        &fixture.store_a,
        &["product-manager"],
    )
    .await;
    let (status, tools, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/ai/tools",
        None,
        &[
            ("cookie", product_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tools}");

    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/ai/tasks",
        None,
        &[
            ("cookie", product_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/ai/tasks",
        Some(json!({"tool": "page.get", "input": {"id": "page_1"}})),
        &[
            ("cookie", product_manager.as_str()),
            ("x-csrf-token", product_manager_csrf.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

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
        "/api/v1/admin/ai/tools",
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
        "/api/v1/admin/ai/tasks",
        None,
        &[
            ("cookie", order_manager.as_str()),
            ("x-store-id", fixture.store_a.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Anonymous requests are rejected before permissions are checked.
    let (status, _, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/ai/tools",
        None,
        &[("x-store-id", fixture.store_a.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tasks_are_isolated_per_store() {
    let fixture = fixture().await;

    let (status, created) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({"tool": "order.get", "input": {"id": "order_1"}})),
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let task_id = created["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin("GET", "/api/v1/admin/ai/tasks", None, &fixture.store_b)
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 0);

    let (status, body) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_b,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
