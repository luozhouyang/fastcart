//! Post-purchase admin flow: returns, claims, exchanges and order edits.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::StoreId;
use fastcart_module_order::{CreateOrderInput, CreateOrderItemInput, OrderService};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    admin_auth: String,
    csrf: String,
    store_id: String,
    order_id: String,
    order_item_id: String,
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
    let order = OrderService::new(db)
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 4,
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

    Fixture {
        app,
        admin_auth,
        csrf,
        store_id,
        order_id: order.id.as_str().to_owned(),
        order_item_id: order.items[0].id.as_str().to_owned(),
        _dir: dir,
    }
}

impl Fixture {
    fn headers(&self) -> Vec<(&str, &str)> {
        vec![
            ("cookie", self.admin_auth.as_str()),
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
async fn admin_can_run_the_full_post_purchase_flow() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let base = format!("/api/v1/admin/orders/{}/returns", fixture.order_id);
    let item = json!({
        "order_item_id": fixture.order_item_id,
        "quantity": 2,
        "reason": "damaged"
    });

    // Return: create -> request -> receive -> write off.
    let (status, created, _) = send(
        &fixture.app,
        "POST",
        &base,
        Some(json!({"reason": "customer request", "items": [item]})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["status"], "draft");
    assert_eq!(created["items"][0]["quantity"], 2);
    let return_id = created["id"].as_str().unwrap().to_owned();
    let return_url = format!("{base}/{return_id}");

    let (status, requested, _) = send(
        &fixture.app,
        "POST",
        &format!("{return_url}/request"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{requested}");
    assert_eq!(requested["status"], "requested");

    let (status, received, _) = send(
        &fixture.app,
        "POST",
        &format!("{return_url}/receive"),
        Some(json!({})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{received}");
    assert_eq!(received["status"], "received");
    assert_eq!(received["items"][0]["received_quantity"], 2);

    let (status, written_off, _) = send(
        &fixture.app,
        "POST",
        &format!("{return_url}/write-off"),
        Some(json!({})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{written_off}");
    assert_eq!(written_off["status"], "written_off");
    assert_eq!(written_off["items"][0]["written_off_quantity"], 2);

    let (status, returns, _) = send(&fixture.app, "GET", &base, None, &headers).await;
    assert_eq!(status, StatusCode::OK, "{returns}");
    assert_eq!(returns.as_array().unwrap().len(), 1);

    // Claim: create -> accept, then an invalid cancel is a conflict.
    let claims = format!("/api/v1/admin/orders/{}/claims", fixture.order_id);
    let (status, claim, _) = send(
        &fixture.app,
        "POST",
        &claims,
        Some(json!({
            "reason": "damaged",
            "items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 1,
                "item_type": "refund"
            }]
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{claim}");
    assert_eq!(claim["status"], "requested");
    let claim_id = claim["id"].as_str().unwrap().to_owned();

    let (status, accepted, _) = send(
        &fixture.app,
        "POST",
        &format!("{claims}/{claim_id}/accept"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["status"], "accepted");

    let (status, conflict, _) = send(
        &fixture.app,
        "POST",
        &format!("{claims}/{claim_id}/cancel"),
        Some(json!({})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "CONFLICT");

    // Exchange: create -> accept with inbound and outbound lines.
    let exchanges = format!("/api/v1/admin/orders/{}/exchanges", fixture.order_id);
    let (status, exchange, _) = send(
        &fixture.app,
        "POST",
        &exchanges,
        Some(json!({
            "reason": "wrong size",
            "inbound_items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 1
            }],
            "outbound_items": [{
                "order_item_id": fixture.order_item_id,
                "quantity": 1,
                "replacement_variant_id": "var_replacement"
            }]
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{exchange}");
    assert_eq!(exchange["status"], "requested");
    assert_eq!(exchange["inbound_items"].as_array().unwrap().len(), 1);
    assert_eq!(exchange["outbound_items"].as_array().unwrap().len(), 1);
    let exchange_id = exchange["id"].as_str().unwrap().to_owned();

    let (status, accepted, _) = send(
        &fixture.app,
        "POST",
        &format!("{exchanges}/{exchange_id}/accept"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{accepted}");
    assert_eq!(accepted["status"], "accepted");

    // Order edit: add a gift line, confirm and observe the recalculated total.
    let edits = format!("/api/v1/admin/orders/{}/edits", fixture.order_id);
    let (status, edit, _) = send(
        &fixture.app,
        "POST",
        &edits,
        Some(json!({
            "reason": "add gift",
            "items": [{
                "action": "add",
                "product_title": "Gift card",
                "sku": "GIFT-1",
                "unit_price": 500,
                "after_quantity": 1
            }]
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{edit}");
    assert_eq!(edit["status"], "draft");
    assert_eq!(edit["before_total"], 4_000);
    assert_eq!(edit["after_total"], 4_500);
    let edit_id = edit["id"].as_str().unwrap().to_owned();

    let (status, confirmed, _) = send(
        &fixture.app,
        "POST",
        &format!("{edits}/{edit_id}/confirm"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(confirmed["status"], "confirmed");

    let (status, order, _) = send(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/orders/{}", fixture.order_id),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{order}");
    assert_eq!(order["total"], 4_500);
    assert_eq!(order["subtotal"], 4_500);
    // The historical item snapshot is unchanged.
    assert_eq!(order["items"].as_array().unwrap().len(), 1);
    assert_eq!(order["items"][0]["quantity"], 4);
}
