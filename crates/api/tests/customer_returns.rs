//! Customer-facing return requests over HTTP.
//!
//! A customer may request a return for one of their own fulfilled orders, list
//! and read only their own submitted returns, and is stopped from stacking open
//! requests or over-returning a line.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{CustomerId, OrderId, StoreId, VariantId};
use fastcart_module_order::{
    CreateOrderInput, CreateOrderItemInput, FulfillmentInput, OrderService,
};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    admin_auth: String,
    admin_csrf: String,
    store_id: String,
    alice: CustomerSession,
    bob: CustomerSession,
    alice_order: OrderRef,
    alice_pending_order: OrderRef,
    bob_order: OrderRef,
    _dir: tempfile::TempDir,
}

struct CustomerSession {
    headers: Vec<(String, String)>,
    id: CustomerId,
}

struct OrderRef {
    id: OrderId,
    item_id: String,
}

impl CustomerSession {
    fn get_headers(&self) -> Vec<(&str, &str)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect()
    }
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
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, store, _) = send(
        &app,
        "POST",
        "/api/v1/platform/stores",
        Some(json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": "shop.example.com"
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = store["id"].as_str().unwrap().to_owned();

    let db = context
        .databases
        .database(&StoreId::from_string(store_id.clone()))
        .await
        .unwrap();

    let alice = register_customer(&app, "alice@example.com").await;
    let bob = register_customer(&app, "bob@example.com").await;

    let orders = OrderService::new(db.clone());
    let alice_order = place_order(&orders, &alice.id, "alice-bought", 4, true).await;
    let alice_pending_order = place_order(&orders, &alice.id, "alice-pending", 2, false).await;
    let bob_order = place_order(&orders, &bob.id, "bob-bought", 3, true).await;

    Fixture {
        app,
        admin_auth,
        admin_csrf: csrf,
        store_id,
        alice,
        bob,
        alice_order,
        alice_pending_order,
        bob_order,
        _dir: dir,
    }
}

/// Places an order directly through the service and optionally ships it.
///
/// A shipped order is confirmed and fully fulfilled so the customer-facing
/// return rule accepts it; an unshipped one stays pending.
async fn place_order(
    orders: &OrderService,
    customer_id: &CustomerId,
    title: &str,
    quantity: i64,
    ship: bool,
) -> OrderRef {
    let variant_id = VariantId::new();
    let order = orders
        .create_order(CreateOrderInput {
            customer_id: Some(customer_id.clone()),
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: title.to_owned(),
                unit_price: 1_000,
                quantity,
                variant_id: Some(variant_id),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000 * quantity,
            total: 1_000 * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    if ship {
        orders.confirm(&order.id).await.unwrap();
        orders
            .fulfill(
                &order.id,
                FulfillmentInput {
                    provider_data: json!({}),
                    ..FulfillmentInput::default()
                },
            )
            .await
            .unwrap();
    }
    OrderRef {
        id: order.id,
        item_id: order.items[0].id.as_str().to_owned(),
    }
}

async fn register_customer(app: &Router, email: &str) -> CustomerSession {
    let (status, body, cookies) = send(
        app,
        "POST",
        "/api/v1/store/customers",
        Some(json!({
            "email": email,
            "password": "password123"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let session = cookie_value(&cookies, "fc_customer_session").expect("customer session");
    let csrf = cookie_value(&cookies, "fc_customer_csrf").expect("customer csrf");
    CustomerSession {
        headers: vec![
            (
                "cookie".to_owned(),
                format!("fc_customer_session={session}; fc_customer_csrf={csrf}"),
            ),
            ("x-csrf-token".to_owned(), csrf),
        ],
        id: CustomerId::from_string(body["customer"]["id"].as_str().unwrap()),
    }
}

impl Fixture {
    fn admin_headers(&self) -> Vec<(&str, &str)> {
        vec![
            ("cookie", self.admin_auth.as_str()),
            ("x-csrf-token", self.admin_csrf.as_str()),
            ("x-store-id", self.store_id.as_str()),
        ]
    }

    fn returns_url(&self, order: &OrderRef) -> String {
        format!(
            "/api/v1/store/customers/me/orders/{}/returns",
            order.id.as_str()
        )
    }

    async fn create_return(
        &self,
        customer: &CustomerSession,
        order: &OrderRef,
        body: Value,
    ) -> (StatusCode, Value, Vec<String>) {
        send(
            &self.app,
            "POST",
            &self.returns_url(order),
            Some(body),
            &customer.get_headers(),
        )
        .await
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
        .header("host", "shop.example.com");
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

#[tokio::test]
async fn customer_can_request_and_read_back_a_return() {
    let fixture = fixture().await;
    let body = json!({
        "reason": "arrived damaged",
        "items": [{
            "order_item_id": fixture.alice_order.item_id,
            "quantity": 2
        }]
    });

    let (status, created, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_order, body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["status"], "requested");
    assert_eq!(created["source"], "customer");
    assert_eq!(created["reason"], "arrived damaged");
    assert_eq!(created["items"][0]["quantity"], 2);
    assert_eq!(
        created["items"][0]["order_item_id"],
        fixture.alice_order.item_id
    );
    // The customer view never exposes the admin-only refund and metadata fields.
    assert!(created.get("refund_id").is_none(), "{created}");
    assert!(created.get("metadata").is_none(), "{created}");
    let return_id = created["id"].as_str().unwrap().to_owned();

    let (status, listed, _) = send(
        &fixture.app,
        "GET",
        &fixture.returns_url(&fixture.alice_order),
        None,
        &fixture.alice.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], return_id);

    let (status, fetched, _) = send(
        &fixture.app,
        "GET",
        &format!("{}/{return_id}", fixture.returns_url(&fixture.alice_order)),
        None,
        &fixture.alice.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{fetched}");
    assert_eq!(fetched, created);

    // The admin list still reports the same record, marked as customer-created.
    let (status, admin_returns, _) = send(
        &fixture.app,
        "GET",
        &format!(
            "/api/v1/admin/orders/{}/returns",
            fixture.alice_order.id.as_str()
        ),
        None,
        &fixture.admin_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{admin_returns}");
    let admin_returns = admin_returns.as_array().unwrap();
    assert_eq!(admin_returns.len(), 1);
    assert_eq!(admin_returns[0]["source"], "customer");
    assert_eq!(admin_returns[0]["status"], "requested");
}

#[tokio::test]
async fn another_customer_cannot_see_the_order_or_its_returns() {
    let fixture = fixture().await;
    let body = json!({
        "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 1}]
    });

    // Requesting a return on someone else's order is a 404, not a 403, so the
    // order's existence does not leak.
    let (status, error, _) = fixture
        .create_return(&fixture.bob, &fixture.alice_order, body)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(error["error"]["code"], "NOT_FOUND");

    let (status, error, _) = send(
        &fixture.app,
        "GET",
        &fixture.returns_url(&fixture.alice_order),
        None,
        &fixture.bob.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    let (status, error, _) = send(
        &fixture.app,
        "GET",
        &format!("{}/ret_dummy", fixture.returns_url(&fixture.alice_order)),
        None,
        &fixture.bob.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    // The anonymous storefront cannot request returns either.
    let (status, body, _) = send(
        &fixture.app,
        "POST",
        &fixture.returns_url(&fixture.alice_order),
        Some(json!({"items": []})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

#[tokio::test]
async fn orders_that_have_not_shipped_are_not_returnable() {
    let fixture = fixture().await;
    let body = json!({
        "items": [{"order_item_id": fixture.alice_pending_order.item_id, "quantity": 1}]
    });

    let (status, error, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_pending_order, body)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["error"]["code"], "CONFLICT");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("pending"), "{message}");
    assert!(message.contains("not_fulfilled"), "{message}");
}

#[tokio::test]
async fn a_second_open_return_is_rejected() {
    let fixture = fixture().await;
    let body = || {
        json!({
            "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 1}]
        })
    };

    let (status, created, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_order, body())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let (status, error, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_order, body())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("open return"), "{message}");

    // Canceling the open return frees the order for a new request.
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/orders/{}/returns/{}/cancel",
            fixture.alice_order.id.as_str(),
            created["id"].as_str().unwrap()
        ),
        Some(json!({"reason": "duplicate"})),
        &fixture.admin_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, second, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_order, body())
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
}

#[tokio::test]
async fn returning_more_than_remains_is_rejected() {
    let fixture = fixture().await;
    let body = json!({
        "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 5}]
    });

    let (status, error, _) = fixture
        .create_return(&fixture.alice, &fixture.alice_order, body)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("only 4 remain available"), "{message}");

    // An admin return already recorded for the line reduces the remainder.
    let (status, admin_return, _) = send(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/orders/{}/returns",
            fixture.alice_order.id.as_str()
        ),
        Some(json!({
            "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 3}]
        })),
        &fixture.admin_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{admin_return}");

    let (status, error, _) = fixture
        .create_return(
            &fixture.alice,
            &fixture.alice_order,
            json!({
                "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 2}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("only 1 remain available"), "{message}");
}

#[tokio::test]
async fn each_customer_sees_only_their_own_returns() {
    let fixture = fixture().await;

    let (status, alice_return, _) = fixture
        .create_return(
            &fixture.alice,
            &fixture.alice_order,
            json!({
                "items": [{"order_item_id": fixture.alice_order.item_id, "quantity": 1}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{alice_return}");

    let (status, bob_return, _) = fixture
        .create_return(
            &fixture.bob,
            &fixture.bob_order,
            json!({
                "items": [{"order_item_id": fixture.bob_order.item_id, "quantity": 1}]
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{bob_return}");

    let (status, alice_list, _) = send(
        &fixture.app,
        "GET",
        &fixture.returns_url(&fixture.alice_order),
        None,
        &fixture.alice.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{alice_list}");
    let alice_list = alice_list.as_array().unwrap();
    assert_eq!(alice_list.len(), 1);
    assert_eq!(alice_list[0]["id"], alice_return["id"]);

    let (status, bob_list, _) = send(
        &fixture.app,
        "GET",
        &fixture.returns_url(&fixture.bob_order),
        None,
        &fixture.bob.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bob_list}");
    let bob_list = bob_list.as_array().unwrap();
    assert_eq!(bob_list.len(), 1);
    assert_eq!(bob_list[0]["id"], bob_return["id"]);

    // A draft return an admin stages is store-side work in progress and stays
    // out of the customer's view.
    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        &format!(
            "/api/v1/admin/orders/{}/returns",
            fixture.bob_order.id.as_str()
        ),
        Some(json!({
            "items": [{"order_item_id": fixture.bob_order.item_id, "quantity": 1}]
        })),
        &fixture.admin_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    assert_eq!(draft["status"], "draft");
    assert_eq!(draft["source"], "admin");

    let (status, bob_list, _) = send(
        &fixture.app,
        "GET",
        &fixture.returns_url(&fixture.bob_order),
        None,
        &fixture.bob.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bob_list}");
    assert_eq!(bob_list.as_array().unwrap().len(), 1);

    // Reading the draft by id is a 404 for the customer as well.
    let (status, error, _) = send(
        &fixture.app,
        "GET",
        &format!(
            "{}/{}",
            fixture.returns_url(&fixture.bob_order),
            draft["id"].as_str().unwrap()
        ),
        None,
        &fixture.bob.get_headers(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
}
