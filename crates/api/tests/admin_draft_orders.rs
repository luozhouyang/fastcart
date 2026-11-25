//! Admin-authored draft orders: creation, conversion and re-taxation.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{CountryCode, StoreId, VariantId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_store::StoreService;
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    app: Router,
    db: StoreDatabase,
    admin_auth: String,
    csrf: String,
    store_id: String,
    _dir: tempfile::TempDir,
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

    Fixture {
        app,
        db,
        admin_auth,
        csrf,
        store_id,
        _dir: dir,
    }
}

async fn configure_tax(db: &StoreDatabase, country: &str, name: &str, rate_bps: i64) {
    let tax = TaxService::new(db.clone());
    let region = tax
        .create_region(CreateTaxRegionInput {
            name: name.to_owned(),
            country_code: CountryCode::new(country).unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: region.id,
        tax_category_id: None,
        name: format!("{name} VAT"),
        rate_bps,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
    .await
    .unwrap();
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

fn draft_request(country: &str) -> Value {
    json!({
        "email": "buyer@example.com",
        "currency_code": "USD",
        "items": [{
            "product_title": "Widget",
            "unit_price": 1_000,
            "quantity": 2,
            "requires_shipping": true
        }],
        "shipping_address": {"country_code": country},
        "billing_address": {"country_code": country}
    })
}

#[tokio::test]
async fn admin_can_author_and_convert_a_draft_order() {
    let fixture = fixture().await;
    let headers = fixture.headers();

    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(draft_request("DE")),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    assert_eq!(draft["status"], "draft");
    assert_eq!(draft["total"], 2_000);
    assert_eq!(draft["status_history"][0]["status"], "draft");
    let order_id = draft["id"].as_str().unwrap();

    let (status, listed, _) = send(
        &fixture.app,
        "GET",
        "/api/v1/admin/orders?status=draft",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["pagination"]["total"], 1);
    assert_eq!(listed["data"][0]["id"], order_id);

    // A draft cannot be fulfilled before it is converted.
    let (status, error, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/fulfill"),
        Some(json!({})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    let (status, converted, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/convert"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{converted}");
    assert_eq!(converted["status"], "pending");

    let (status, confirmed, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/confirm"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    assert_eq!(confirmed["status"], "confirmed");
}

#[tokio::test]
async fn changing_the_shipping_address_recalculates_draft_tax() {
    let fixture = fixture().await;
    configure_tax(&fixture.db, "DE", "Germany", 1_000).await;
    configure_tax(&fixture.db, "FR", "France", 2_000).await;
    let headers = fixture.headers();

    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(draft_request("DE")),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    assert_eq!(draft["tax_total"], 200);
    assert_eq!(draft["total"], 2_200);
    let order_id = draft["id"].as_str().unwrap();

    let (status, updated, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/shipping-address"),
        Some(json!({"shipping_address": {"country_code": "FR"}})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["shipping_address"]["country_code"], "FR");
    assert_eq!(updated["tax_total"], 400);
    assert_eq!(updated["total"], 2_400);
    assert_eq!(updated["items"][0]["tax_total"], 400);
    assert_eq!(updated["items"][0]["total"], 2_400);
}

#[tokio::test]
async fn a_placed_order_address_cannot_be_re_taxed_through_the_draft_route() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(draft_request("DE")),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    let order_id = draft["id"].as_str().unwrap();
    let (status, _, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/convert"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, error, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/shipping-address"),
        Some(json!({"shipping_address": {"country_code": "FR"}})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
}

#[tokio::test]
async fn draft_creation_requires_an_order_email_and_lines() {
    let fixture = fixture().await;
    let headers = fixture.headers();
    let (status, error, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(json!({
            "email": "not-an-email",
            "currency_code": "USD",
            "items": [{"product_title": "Widget", "unit_price": 100, "quantity": 1}]
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    let (status, error, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(json!({
            "email": "buyer@example.com",
            "currency_code": "USD",
            "items": []
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

#[tokio::test]
async fn draft_lines_follow_the_store_pricing_setting() {
    let fixture = fixture().await;
    configure_tax(&fixture.db, "DE", "Germany", 1_900).await;
    StoreService::new(fixture.db.clone())
        .set_setting("prices_include_tax", "true")
        .await
        .unwrap();
    let headers = fixture.headers();

    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(draft_request("DE")),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    assert_eq!(draft["items"][0]["is_tax_inclusive"], true);
    // 2000 gross at 19%: the tax is inside the price, not on top of it.
    assert_eq!(draft["items"][0]["tax_total"], 319);
    assert_eq!(draft["items"][0]["total"], 2_000);
    assert_eq!(draft["tax_total"], 319);
    assert_eq!(draft["total"], 2_000);

    // An explicit flag still wins over the store convention.
    let mut request = draft_request("DE");
    request["items"][0]["is_tax_inclusive"] = json!(false);
    let (status, exclusive, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(request),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{exclusive}");
    assert_eq!(exclusive["items"][0]["is_tax_inclusive"], false);
    assert_eq!(exclusive["items"][0]["tax_total"], 380);
    assert_eq!(exclusive["items"][0]["total"], 2_380);
    assert_eq!(exclusive["total"], 2_380);
}

#[tokio::test]
async fn converting_a_draft_takes_its_stock() {
    let fixture = fixture().await;
    let inventory = InventoryService::new(fixture.db.clone());
    let catalog = CatalogService::new(fixture.db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Managed widget".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant = VariantId::from_string(product.variants[0].id.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Warehouse".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 10, "restock", None)
        .await
        .unwrap();
    let headers = fixture.headers();

    let (status, draft, _) = send(
        &fixture.app,
        "POST",
        "/api/v1/admin/orders",
        Some(json!({
            "email": "buyer@example.com",
            "currency_code": "USD",
            "items": [{
                "variant_id": variant.as_str(),
                "product_title": "Managed widget",
                "unit_price": 1_000,
                "quantity": 2,
                "requires_shipping": true
            }],
            "shipping_address": {"country_code": "US"},
            "billing_address": {}
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    let order_id = draft["id"].as_str().unwrap();
    assert_eq!(inventory.variant_availability(&variant).await.unwrap(), 10);

    let (status, converted, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/convert"),
        None,
        &headers,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{converted}");
    assert_eq!(converted["status"], "pending");
    assert_eq!(
        inventory.variant_availability(&variant).await.unwrap(),
        8,
        "conversion commits the units the order promises"
    );
    let reservations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE order_id = ? AND status = 'committed'",
    )
    .bind(order_id)
    .fetch_one(fixture.db.pool())
    .await
    .unwrap();
    assert_eq!(reservations, 1);
}
