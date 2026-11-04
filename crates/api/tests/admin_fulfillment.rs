//! Admin fulfillment flow: provider registry integration and idempotent-ish
//! remaining-quantity handling across partial shipments.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{StoreId, VariantId};
use fastcart_module_order::{CreateOrderInput, CreateOrderItemInput, OrderService};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    headers: Vec<(String, String)>,
    store_id: String,
    order_id: String,
    order_item_id: String,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn headers(&self) -> Vec<(&str, &str)> {
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
            "password": "a-very-long-password"
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let headers = vec![
        (
            "cookie".to_owned(),
            format!("fc_admin_session={session}; fc_csrf={csrf}"),
        ),
        ("x-csrf-token".to_owned(), csrf),
    ];

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
            ("cookie", headers[0].1.as_str()),
            ("x-csrf-token", headers[1].1.as_str()),
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
    let service = OrderService::new(db);
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                variant_id: Some(VariantId::from_string("var_widget")),
                product_title: "Widget".to_owned(),
                variant_title: Some("Default".to_owned()),
                unit_price: 1_000,
                quantity: 4,
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 4_000,
            total: 4_000,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let order = service.confirm(&order.id).await.unwrap();

    Fixture {
        app,
        headers,
        store_id,
        order_id: order.id.as_str().to_owned(),
        order_item_id: order.items[0].id.as_str().to_owned(),
        _dir: dir,
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

impl Fixture {
    async fn fulfill(&self, body: Value) -> (StatusCode, Value) {
        let mut headers = self.headers();
        headers.push(("x-store-id", self.store_id.as_str()));
        let (status, value, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/admin/orders/{}/fulfill", self.order_id),
            Some(body),
            &headers,
        )
        .await;
        (status, value)
    }

    async fn order(&self) -> Value {
        let mut headers = self.headers();
        headers.push(("x-store-id", self.store_id.as_str()));
        let (status, value, _) = send(
            &self.app,
            "GET",
            &format!("/api/v1/admin/orders/{}", self.order_id),
            None,
            &headers,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        value
    }
}

#[tokio::test]
async fn admin_fulfillment_creates_a_provider_fulfillment() {
    let fixture = fixture().await;

    let (status, order) = fixture
        .fulfill(json!({
            "provider_id": "manual",
            "items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 4
            }],
            "tracking_numbers": ["TRACK-1"]
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{order}");

    let fulfillment = &order["fulfillments"][0];
    assert_eq!(fulfillment["provider_id"], "manual");
    let provider_fulfillment_id = fulfillment["provider_fulfillment_id"]
        .as_str()
        .expect("provider fulfillment id should be persisted");
    assert!(
        provider_fulfillment_id.starts_with("ful_"),
        "unexpected provider fulfillment id: {provider_fulfillment_id}"
    );
    assert_eq!(fulfillment["tracking_numbers"][0], "TRACK-1");
    assert_eq!(fulfillment["items"][0]["quantity"], 4);
    assert_eq!(order["fulfillment_status"], "fulfilled");
}

#[tokio::test]
async fn unknown_provider_is_rejected_without_recording_a_fulfillment() {
    let fixture = fixture().await;

    let (status, error) = fixture
        .fulfill(json!({
            "provider_id": "not-a-provider",
            "items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 1
            }]
        }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    let order = fixture.order().await;
    assert_eq!(order["fulfillments"].as_array().unwrap().len(), 0);
    assert_eq!(order["fulfillment_status"], "not_fulfilled");
}

#[tokio::test]
async fn partial_then_remaining_fulfillment_uses_explicit_and_computed_items() {
    let fixture = fixture().await;

    let (status, partial) = fixture
        .fulfill(json!({
            "provider_id": "manual",
            "items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 2
            }],
            "partial": true
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{partial}");
    assert_eq!(partial["fulfillment_status"], "partially_fulfilled");
    assert_eq!(partial["fulfillments"][0]["items"][0]["quantity"], 2);
    assert!(
        partial["fulfillments"][0]["provider_fulfillment_id"]
            .as_str()
            .unwrap()
            .starts_with("ful_")
    );

    // No explicit items: the handler must compute the two remaining units.
    let (status, complete) = fixture.fulfill(json!({"provider_id": "manual"})).await;
    assert_eq!(status, StatusCode::OK, "{complete}");
    assert_eq!(complete["fulfillment_status"], "fulfilled");
    assert_eq!(complete["fulfillments"].as_array().unwrap().len(), 2);
    assert_eq!(complete["fulfillments"][1]["items"][0]["quantity"], 2);
    assert!(
        complete["fulfillments"][1]["provider_fulfillment_id"]
            .as_str()
            .unwrap()
            .starts_with("ful_")
    );
}
