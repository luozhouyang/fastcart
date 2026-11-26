//! Route-level coverage for `/api/v1/admin/returns`.
//!
//! Returns were previously reachable only per order. The store-wide list is
//! the operational view across every order, so these tests cover its
//! permission gate, pagination envelope, status filter and order context.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{Metadata, Pagination, StoreId};
use fastcart_module_order::{
    CreateOrderInput, CreateOrderItemInput, CreateReturnInput, Order, OrderService,
    ReturnItemInput, ReturnStatus, StoreReturnFilter,
};
use fastcart_platform::{CreateUserInput, PlatformStore};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    context: Arc<AppContext>,
    db: StoreDatabase,
    owner_auth: String,
    csrf: String,
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
            "password": "hunter2hunter2"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "platform setup");
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let owner_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, store, _) = send(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Returns",
            "slug": "returns",
            "primary_domain": "returns.example.com"
        })),
        &[
            ("cookie", owner_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = StoreId::from_string(store["id"].as_str().unwrap());

    let db = context.databases.database(&store_id).await.unwrap();

    Fixture {
        app,
        context,
        db,
        owner_auth,
        csrf,
        store_id,
        _dir: dir,
    }
}

impl Fixture {
    fn headers(&self) -> Vec<(&str, &str)> {
        vec![
            ("cookie", self.owner_auth.as_str()),
            ("x-csrf-token", self.csrf.as_str()),
            ("x-store-id", self.store_id.as_str()),
        ]
    }
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, Value, Vec<String>) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "returns.example.com");
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

/// Places a one-line order and creates a return for it.
async fn order_with_return(
    db: &StoreDatabase,
    email: &str,
    requested: bool,
) -> (Order, ReturnStatus) {
    let service = OrderService::new(db.clone());
    let order = service
        .create_order(CreateOrderInput {
            email: email.to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 2,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 2_000,
            total: 2_000,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let created = service
        .create_return(&order.id, return_input(&order, "damaged"))
        .await
        .unwrap();
    if requested {
        service
            .request_return(&order.id, &created.id)
            .await
            .unwrap();
    }
    (order, return_status(requested))
}

fn return_status(requested: bool) -> ReturnStatus {
    if requested {
        ReturnStatus::Requested
    } else {
        ReturnStatus::Draft
    }
}

fn return_input(order: &Order, reason: &str) -> CreateReturnInput {
    CreateReturnInput {
        reason: Some(reason.to_owned()),
        reason_id: None,
        refund_id: None,
        metadata: Metadata::default(),
        items: vec![ReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity: 1,
            reason: None,
        }],
    }
}

/// Signs up a user holding `roles` in the store and returns their cookie pair
/// plus the CSRF token the pair carries.
async fn member_auth(fixture: &Fixture, email: &str, roles: &[&str]) -> (String, String) {
    let member = fixture
        .context
        .auth
        .register(CreateUserInput::new(email), "member-password")
        .await
        .unwrap();
    fixture
        .context
        .platform
        .upsert_membership(&member.id, &fixture.store_id, roles)
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

#[tokio::test]
async fn store_returns_list_across_orders_with_pagination() {
    let fixture = fixture().await;
    let (first_order, first_status) =
        order_with_return(&fixture.db, "first@example.com", false).await;
    let (second_order, second_status) =
        order_with_return(&fixture.db, "second@example.com", true).await;
    assert_eq!(first_status, ReturnStatus::Draft);
    assert_eq!(second_status, ReturnStatus::Requested);

    // One page holds both returns and reports the envelope the admin table
    // needs: data plus limit/offset/total.
    let (status, page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?limit=10&offset=0",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["limit"], 10);
    assert_eq!(page["pagination"]["offset"], 0);
    assert_eq!(page["pagination"]["total"], 2);
    let rows = page["data"].as_array().expect("data array");
    assert_eq!(rows.len(), 2);
    let order_ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| row["order_id"].as_str())
        .collect();
    assert!(order_ids.contains(&first_order.id.as_str()), "{page}");
    assert!(order_ids.contains(&second_order.id.as_str()), "{page}");
    let first_row = rows
        .iter()
        .find(|row| row["order_id"] == first_order.id.as_str())
        .expect("first order return is listed");
    assert_eq!(
        first_row["order_display_number"],
        first_order.display_number
    );
    assert_eq!(first_row["customer_email"], "first@example.com");
    assert_eq!(first_row["status"], "draft");
    assert_eq!(first_row["reason"], "damaged");
    assert_eq!(first_row["item_count"], 1);
    assert!(first_row["created_at"].as_i64().is_some());
    assert!(first_row["updated_at"].as_i64().is_some());

    // The second page with the same page size is empty, keeping the envelope.
    let (status, second_page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?limit=10&offset=10",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_page}");
    assert_eq!(second_page["data"].as_array().unwrap().len(), 0);
    assert_eq!(second_page["pagination"]["total"], 2);
    assert_eq!(second_page["pagination"]["offset"], 10);

    // Two single-row pages cover both returns.
    let (status, first_page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?limit=1&offset=0",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first_page}");
    assert_eq!(first_page["data"].as_array().unwrap().len(), 1);
    let (status, second_page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?limit=1&offset=1",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_page}");
    assert_eq!(second_page["data"].as_array().unwrap().len(), 1);
    let paged_ids = [
        first_page["data"][0]["order_id"].as_str().unwrap(),
        second_page["data"][0]["order_id"].as_str().unwrap(),
    ];
    assert!(paged_ids.contains(&first_order.id.as_str()));
    assert!(paged_ids.contains(&second_order.id.as_str()));
}

#[tokio::test]
async fn store_returns_list_filters_by_status() {
    let fixture = fixture().await;
    order_with_return(&fixture.db, "draft@example.com", false).await;
    let (requested_order, _) = order_with_return(&fixture.db, "requested@example.com", true).await;

    let (status, page, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?status=requested",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1);
    assert_eq!(page["data"][0]["order_id"], requested_order.id.as_str());
    assert_eq!(page["data"][0]["status"], "requested");

    let (status, empty, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?status=canceled",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["pagination"]["total"], 0);
    assert_eq!(empty["data"].as_array().unwrap().len(), 0);

    // An unknown status is a validation error rather than an empty page.
    let (status, invalid, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns?status=nonsense",
        None,
        &fixture.headers(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}");
}

#[tokio::test]
async fn store_returns_list_enforces_orders_read_permission() {
    let fixture = fixture().await;

    // `content-editor` does not hold `orders.read`.
    let (editor_auth, _) = member_auth(&fixture, "editor@example.com", &["content-editor"]).await;
    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns",
        None,
        &[
            ("cookie", editor_auth.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    // A support agent holds `orders.read` and sees the same empty list.
    let (support_auth, _) = member_auth(&fixture, "support@example.com", &["support-agent"]).await;
    let (status, body, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/returns",
        None,
        &[
            ("cookie", support_auth.as_str()),
            ("x-store-id", fixture.store_id.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["pagination"]["total"], 0);

    // Anonymous requests are rejected before the permission check.
    let (status, _, _) = send(&fixture.app, "GET", "/api/v1/admin/returns", None, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// The service is also exercised directly so the API test does not depend on
/// the route filter mapping for its own expectations.
#[tokio::test]
async fn store_returns_service_filter_matches_the_route() {
    let fixture = fixture().await;
    order_with_return(&fixture.db, "draft@example.com", false).await;
    order_with_return(&fixture.db, "requested@example.com", true).await;

    let page = OrderService::new(fixture.db.clone())
        .list_store_returns(
            Pagination::default(),
            StoreReturnFilter {
                status: Some(ReturnStatus::Requested),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 1);
    assert_eq!(page.data[0].status, ReturnStatus::Requested);
}
