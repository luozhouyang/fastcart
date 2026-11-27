//! Route-level coverage for `/api/v1/admin/ai`.
//!
//! These tests drive the real HTTP surface: authentication, the
//! `settings.read` / `settings.write` permissions, the tool catalog, task
//! creation, deterministic tool execution and the approval-gated state
//! machine. Read-only tools run when the task is created; mutating tools run
//! only after approval. No model is called.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{Metadata, StoreId};
use fastcart_module_content::{ContentService, CreatePageInput, Page};
use fastcart_module_order::{CreateOrderInput, CreateOrderItemInput, Order, OrderService};
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

    /// Opens the store database used by the running router.
    async fn db(&self, store: &StoreId) -> StoreDatabase {
        self.context.databases.database(store).await.unwrap()
    }

    /// Creates a page through the content service.
    async fn create_page(&self, store: &StoreId, title: &str, handle: &str) -> Page {
        ContentService::new(self.db(store).await)
            .create_page(CreatePageInput {
                title: title.to_owned(),
                handle: Some(handle.to_owned()),
                page_type: Some("custom".to_owned()),
                body: json!({
                    "schema_version": 1,
                    "content": [{
                        "type": "RichText",
                        "props": {"id": "rich-1", "body": "hello"}
                    }]
                }),
                seo: json!({"title": title}),
                metadata: Metadata::empty(),
            })
            .await
            .unwrap()
    }

    /// Seeds one order through the order service.
    async fn seed_order(&self, store: &StoreId) -> Order {
        OrderService::new(self.db(store).await)
            .create_order(CreateOrderInput {
                email: "buyer@example.com".to_owned(),
                currency_code: "USD".to_owned(),
                items: vec![CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    requires_shipping: false,
                    ..CreateOrderItemInput::default()
                }],
                subtotal: 2_000,
                total: 2_000,
                shipping_address: json!({}),
                billing_address: json!({}),
                ..CreateOrderInput::default()
            })
            .await
            .unwrap()
    }

    /// Creates a task through the HTTP surface and returns its id.
    async fn create_task(&self, tool: &str, input: Value) -> (StatusCode, Value) {
        self.admin(
            "POST",
            "/api/v1/admin/ai/tasks",
            Some(json!({"tool": tool, "input": input})),
            &self.store_a,
        )
        .await
    }

    /// Approves a task through the HTTP surface.
    async fn approve(&self, task_id: &str) -> (StatusCode, Value) {
        self.admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{task_id}/approve"),
            None,
            &self.store_a,
        )
        .await
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
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;

    let (status, created) = fixture
        .create_task(
            "page.update",
            json!({"id": page.id.as_str(), "title": "Draft"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert!(created["id"].as_str().unwrap().starts_with("task_"));
    assert_eq!(created["tool"], "page.update");
    assert_eq!(created["status"], "pending", "mutations wait for approval");
    assert_eq!(
        created["input"],
        json!({"id": page.id.as_str(), "title": "Draft"})
    );
    assert!(created["output"].is_null());
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

    let (status, page_list) = fixture
        .admin("GET", "/api/v1/admin/ai/tasks", None, &fixture.store_a)
        .await;
    assert_eq!(status, StatusCode::OK, "{page_list}");
    assert_eq!(page_list["pagination"]["total"], 1);
    assert_eq!(page_list["data"][0]["id"], task_id.as_str());

    // Approval claims the task, executes it and records the terminal state.
    let (status, approved) = fixture.approve(&task_id).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "succeeded", "{approved}");
    assert_eq!(approved["output"]["title"], "Draft");
    assert!(approved["error"].is_null());

    // A terminal task cannot be approved a second time.
    let (status, conflict) = fixture.approve(&task_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "CONFLICT");
    assert_eq!(conflict["error"]["details"]["from"], "succeeded");
    assert_eq!(conflict["error"]["details"]["to"], "running");

    // Rejection keeps cancelling a task that has not run.
    let (status, pending) = fixture
        .create_task("page.publish", json!({"id": page.id.as_str()}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    let publish_id = pending["id"].as_str().unwrap().to_owned();
    let (status, rejected) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/ai/tasks/{publish_id}/reject"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    assert_eq!(rejected["status"], "cancelled");

    let (status, conflict) = fixture.approve(&publish_id).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["details"]["from"], "cancelled");
    assert_eq!(conflict["error"]["details"]["to"], "running");

    // The rejected task is still recorded with its final status.
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{publish_id}"),
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
        .create_task("page.publish", json!({"id": "page_1"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["status"], "pending");
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
async fn page_get_executes_immediately_and_persists_output() {
    let fixture = fixture().await;
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;

    let (status, task) = fixture
        .create_task("page.get", json!({"id": page.id.as_str()}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "succeeded", "read tools run on creation");
    assert_eq!(task["output"]["id"], page.id.as_str());
    assert_eq!(task["output"]["title"], "About");
    assert_eq!(task["output"]["handle"], "about");
    assert!(task["error"].is_null());

    let task_id = task["id"].as_str().unwrap();
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["status"], "succeeded");
    assert_eq!(fetched["output"]["id"], page.id.as_str());
}

#[tokio::test]
async fn page_update_requires_approval_and_updates_the_draft() {
    let fixture = fixture().await;
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;

    let (status, task) = fixture
        .create_task(
            "page.update",
            json!({"id": page.id.as_str(), "title": "Updated"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "pending");
    let task_id = task["id"].as_str().unwrap().to_owned();

    // The draft is untouched until the task is approved.
    let stored = ContentService::new(fixture.db(&fixture.store_a).await)
        .get_page(&page.id)
        .await
        .unwrap();
    assert_eq!(stored.title, "About");
    assert_eq!(stored.revision, 1);

    let (status, approved) = fixture.approve(&task_id).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "succeeded", "{approved}");
    assert_eq!(approved["output"]["title"], "Updated");
    assert_eq!(
        approved["output"]["handle"], "about",
        "omitted fields keep their current value"
    );
    assert_eq!(approved["output"]["revision"], 2);

    let stored = ContentService::new(fixture.db(&fixture.store_a).await)
        .get_page(&page.id)
        .await
        .unwrap();
    assert_eq!(stored.title, "Updated");
    assert_eq!(stored.handle, "about");
    assert_eq!(stored.revision, 2);
}

#[tokio::test]
async fn page_publish_through_approval_publishes_the_page() {
    let fixture = fixture().await;
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;

    let (status, task) = fixture
        .create_task("page.publish", json!({"id": page.id.as_str()}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "pending");
    let task_id = task["id"].as_str().unwrap().to_owned();

    let (status, approved) = fixture.approve(&task_id).await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    assert_eq!(approved["status"], "succeeded", "{approved}");
    assert_eq!(approved["output"]["status"], "published");

    let stored = ContentService::new(fixture.db(&fixture.store_a).await)
        .get_page(&page.id)
        .await
        .unwrap();
    assert_eq!(stored.status.as_str(), "published");
    assert!(stored.published_body.is_some());
}

#[tokio::test]
async fn block_list_types_returns_the_available_block_types() {
    let fixture = fixture().await;

    // Enabled plugin contributions are read from the same cached catalog the
    // admin component route uses, so no HTTP plugin installation is needed.
    let manifest = json!({
        "schema_version": "1",
        "id": "demo",
        "name": "Demo Plugin",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": {"engine": "workerd", "compatibility_date": "2026-11-01"},
        "entry": {"type": "worker", "path": "index.js"},
        "capabilities": {
            "data": ["catalog.read"],
            "storefront": ["storefront.component"]
        },
        "provides": [{
            "kind": "storefront_component",
            "id": "hero",
            "title": "Hero banner"
        }]
    });
    let now = fastcart_kernel::now_timestamp();
    sqlx::query(
        "INSERT INTO plugin_installations (id, plugin_id, digest, version, manifest_json, \
         enabled, settings_json, settings_revision, granted_scopes, installed_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 1, '{}', 0, '[]', ?, ?)",
    )
    .bind("plugin_install_demo")
    .bind("demo")
    .bind("sha256:demo")
    .bind("1.0.0")
    .bind(manifest.to_string())
    .bind(now)
    .bind(now)
    .execute(fixture.db(&fixture.store_a).await.pool())
    .await
    .unwrap();

    let (status, task) = fixture.create_task("block.list_types", json!({})).await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "succeeded", "{task}");
    let types = task["output"].as_array().expect("block type array");
    assert!(
        types.iter().any(|value| value == "plugin:demo:hero"),
        "plugin components use the page editor namespace: {types:?}"
    );
}

#[tokio::test]
async fn order_get_returns_a_seeded_order() {
    let fixture = fixture().await;
    let order = fixture.seed_order(&fixture.store_a).await;

    let (status, task) = fixture
        .create_task("order.get", json!({"id": order.id.as_str()}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "succeeded", "{task}");
    assert_eq!(task["output"]["id"], order.id.as_str());
    assert_eq!(task["output"]["email"], "buyer@example.com");
    assert_eq!(task["output"]["total"], 2_000);
    assert_eq!(task["output"]["items"].as_array().unwrap().len(), 1);

    // A missing order is a structured failure, not a panic or a silent success.
    let (status, task) = fixture
        .create_task("order.get", json!({"id": "order_missing"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "failed");
    assert!(task["output"].is_null());
    let error: Value = serde_json::from_str(task["error"].as_str().unwrap()).unwrap();
    assert_eq!(error["code"], "NOT_FOUND", "{task}");
    assert!(
        error["message"].as_str().unwrap().contains("order"),
        "{task}"
    );
}

#[tokio::test]
async fn failed_execution_persists_a_structured_error() {
    let fixture = fixture().await;

    let (status, task) = fixture
        .create_task("page.get", json!({"id": "page_missing"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    assert_eq!(task["status"], "failed");
    assert!(task["output"].is_null());
    let error: Value = serde_json::from_str(task["error"].as_str().unwrap()).unwrap();
    assert_eq!(error["code"], "NOT_FOUND");
    assert!(
        error["message"].as_str().unwrap().contains("page_missing"),
        "{task}"
    );

    let task_id = task["id"].as_str().unwrap();
    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["status"], "failed");
    assert_eq!(fetched["error"], task["error"]);
    assert!(fetched["output"].is_null());
}

#[tokio::test]
async fn concurrent_approvals_execute_a_mutation_once() {
    let fixture = fixture().await;
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;
    let (status, task) = fixture
        .create_task(
            "page.update",
            json!({"id": page.id.as_str(), "title": "Once"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{task}");
    let task_id = task["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/admin/ai/tasks/{task_id}/approve");

    let headers_a = [
        ("cookie", fixture.owner_auth.as_str()),
        ("x-csrf-token", fixture.csrf.as_str()),
        ("x-store-id", fixture.store_a.as_str()),
    ];
    let headers_b = headers_a;
    let first = send(&fixture.app, "POST", &uri, None, &headers_a);
    let second = send(&fixture.app, "POST", &uri, None, &headers_b);
    let ((first_status, first_body, _), (second_status, second_body, _)) =
        tokio::join!(first, second);

    let mut statuses = [first_status, second_status];
    statuses.sort_by_key(|status| status.as_u16());
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "exactly one approval claims the task: {first_body} / {second_body}"
    );

    // Exactly one execution: the update incremented the revision once.
    let stored = ContentService::new(fixture.db(&fixture.store_a).await)
        .get_page(&page.id)
        .await
        .unwrap();
    assert_eq!(stored.title, "Once");
    assert_eq!(stored.revision, 2);

    let (status, fetched) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/ai/tasks/{task_id}"),
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched["status"], "succeeded");
    assert_eq!(fetched["output"]["title"], "Once");
}

#[tokio::test]
async fn unknown_tools_and_invalid_input_are_rejected() {
    let fixture = fixture().await;

    let (status, body) = fixture
        .create_task("page.delete", json!({"id": "page_1"}))
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
        .create_task("page.get", json!(["not", "an", "object"]))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Inputs are validated against the registry's JSON Schema.
    let (status, body) = fixture.create_task("page.get", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "VALIDATION_ERROR");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("input is invalid"),
        "{body}"
    );

    let (status, body) = fixture
        .create_task("page.get", json!({"id": "page_1", "extra": true}))
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
        .create_task("order.get", json!({"id": "order_1"}))
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

#[tokio::test]
async fn a_task_abandoned_by_a_crashed_runner_is_reclaimed_after_the_lease() {
    let fixture = fixture().await;
    let page = fixture
        .create_page(&fixture.store_a, "About", "about")
        .await;
    let db = fixture.db(&fixture.store_a).await;

    // A runner claimed this task and then died: the row is `running` and its
    // start is older than the execution lease. Only the claim is simulated;
    // the tool input is a normal update.
    let now = fastcart_kernel::now_timestamp();
    let stale_started = now - 3_600;
    sqlx::query(
        "INSERT INTO ai_tasks (id, tool, input_json, status, error, created_by, created_at, \
         updated_at, started_at) VALUES ('task_stale', 'page.update', ?, 'running', NULL, NULL, \
         ?, ?, ?)",
    )
    .bind(json!({"id": page.id.as_str(), "title": "Recovered"}).to_string())
    .bind(now)
    .bind(now)
    .bind(stale_started)
    .execute(db.pool())
    .await
    .unwrap();

    let (status, task) = fixture
        .admin(
            "POST",
            "/api/v1/admin/ai/tasks/task_stale/approve",
            None,
            &fixture.store_a,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{task}");
    assert_eq!(task["status"], "succeeded");
    assert_eq!(task["output"]["title"], "Recovered");

    let stored = ContentService::new(db).get_page(&page.id).await.unwrap();
    assert_eq!(stored.title, "Recovered");
}
