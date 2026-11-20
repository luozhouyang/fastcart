//! Storefront HTTP coverage of a checkout's payment collection.
//!
//! The collection is the money a checkout owes; combination payments are the
//! reason it exists. This test drives the whole path over real HTTP — open a
//! checkout, split its payment onto a second session, complete it — and checks
//! the money that moved, so the capability is proven reachable from the
//! storefront API rather than from the module service alone.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{CurrencyCode, StoreId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";

/// A store with one published product priced at `amount`, an enabled shipping
/// option, and the HTTP surface of the running application.
struct Fixture {
    app: Router,
    db: StoreDatabase,
    variant_id: String,
    shipping_option_id: String,
    _context: Arc<AppContext>,
    _dir: tempfile::TempDir,
}

async fn fixture(amount: i64) -> Fixture {
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

    // Bootstrap an owner and a store through the platform API.
    let (status, _, cookies) = send(
        &app,
        "POST",
        "/api/v1/platform/auth/setup",
        Some(json!({"email": "owner@example.com", "password": "a-very-long-password"})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "platform setup");
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
            "primary_domain": HOST
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = StoreId::from_string(store["id"].as_str().unwrap());

    // Seed the store database through module services; everything a buyer does
    // goes over HTTP.
    let db = context.databases.database(&store_id).await.unwrap();
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Hoodie".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("HOODIE-L".to_owned()),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.to_string();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(
            &variant_id,
            CurrencyCode::new("USD").unwrap(),
            amount,
        ))
        .await
        .unwrap();
    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    let option = shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Standard".to_owned(),
            amount_minor: 0,
            currency: CurrencyCode::new("USD").unwrap(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    Fixture {
        app,
        db,
        variant_id,
        shipping_option_id: option.id.to_string(),
        _context: context,
        _dir: dir,
    }
}

impl Fixture {
    /// Opens a cart with the seeded variant and a checkout for it over HTTP.
    async fn open_checkout(&self) -> (String, String) {
        let host = [("host", HOST)];
        let (status, cart, _) = send(
            &self.app,
            "POST",
            "/api/v1/store/carts",
            Some(json!({"currency_code": "USD"})),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{cart}");
        let cart_id = cart["cart"]["id"].as_str().unwrap().to_owned();

        let (status, cart, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/carts/{cart_id}/items"),
            Some(json!({
                "variant_id": self.variant_id,
                "quantity": 1,
                "country_code": "US"
            })),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cart}");

        let (status, cart, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/carts/{cart_id}/shipping-method"),
            Some(json!({
                "shipping_option_id": self.shipping_option_id,
                "country_code": "US"
            })),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cart}");

        let (status, checkout, _) = send(
            &self.app,
            "POST",
            "/api/v1/store/checkouts",
            Some(json!({
                "cart_id": cart_id,
                "email": "buyer@example.com",
                "payment_provider": "manual",
                "country_code": "US"
            })),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{checkout}");
        assert_eq!(
            checkout["checkout"]["status"], "payment_pending",
            "a fresh checkout is waiting for its payment"
        );
        let checkout_id = checkout["checkout"]["id"].as_str().unwrap().to_owned();
        (cart_id, checkout_id)
    }

    async fn get_collection(&self, checkout_id: &str) -> (StatusCode, Value) {
        let (status, body, _) = send(
            &self.app,
            "GET",
            &format!("/api/v1/store/checkouts/{checkout_id}/payment-collection"),
            None,
            &[("host", HOST)],
        )
        .await;
        (status, body)
    }

    async fn split_session(&self, checkout_id: &str, body: Value) -> (StatusCode, Value) {
        let (status, body, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/checkouts/{checkout_id}/payment-collection/sessions"),
            Some(body),
            &[("host", HOST)],
        )
        .await;
        (status, body)
    }

    async fn complete(&self, checkout_id: &str) -> (StatusCode, Value) {
        let (status, body, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/checkouts/{checkout_id}/complete"),
            Some(json!({"country_code": "US"})),
            &[("host", HOST)],
        )
        .await;
        (status, body)
    }

    fn session_amounts(&self, collection: &Value) -> Vec<i64> {
        collection["payment_sessions"]
            .as_array()
            .expect("payment_sessions is a list")
            .iter()
            .map(|session| {
                session["amount"]
                    .as_i64()
                    .expect("session amount is an integer")
            })
            .collect()
    }
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

/// The payments recorded for the store, oldest first.
async fn recorded_payments(fixture: &Fixture) -> Vec<(String, Option<String>, i64, String)> {
    sqlx::query_as("SELECT id, order_id, amount, status FROM payments ORDER BY amount ASC")
        .fetch_all(fixture.db.pool())
        .await
        .expect("read payments")
}

#[tokio::test]
async fn a_combination_payment_completes_through_the_storefront_api() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;

    // The checkout opened a collection holding the one session it was created
    // with, and the collection is not yet anybody's order.
    let (status, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{collection}");
    assert_eq!(collection["payment_collection"]["amount"], 1_500);
    assert_eq!(collection["payment_collection"]["currency_code"], "USD");
    assert_eq!(collection["payment_collection"]["status"], "open");
    assert_eq!(collection["payment_collection"]["checkout_id"], checkout_id);
    assert!(collection["payment_collection"]["order_id"].is_null());
    assert_eq!(fixture.session_amounts(&collection), vec![1_500]);

    // The second session takes 400 of the money the checkout's own session
    // holds, which is the split a combination payment needs.
    let (status, split) = fixture
        .split_session(
            &checkout_id,
            json!({"provider": "manual", "amount": 400, "currency_code": "USD"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{split}");
    assert_eq!(split["payment_session"]["amount"], 400);
    assert_eq!(split["payment_session"]["currency_code"], "USD");
    assert_eq!(split["payment_session"]["provider"], "manual");
    assert_eq!(split["payment_session"]["checkout_id"], checkout_id);
    let split_session_id = split["payment_session"]["id"].as_str().unwrap().to_owned();

    let (status, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{collection}");
    assert_eq!(
        fixture.session_amounts(&collection),
        vec![1_100, 400],
        "the checkout's session keeps the money the split did not take"
    );
    assert_eq!(
        fixture.session_amounts(&collection).iter().sum::<i64>(),
        collection["payment_collection"]["amount"].as_i64().unwrap(),
        "the sessions still cover the collection exactly"
    );

    // Completing captures both sessions and places one order for the cart.
    let (status, completed) = fixture.complete(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["order"]["total"], 1_500);
    assert_eq!(completed["order"]["payment_status"], "captured");
    let order_id = completed["order"]["id"].as_str().unwrap().to_owned();

    let payments = recorded_payments(&fixture).await;
    assert_eq!(payments.len(), 2, "both sessions were captured");
    assert_eq!(
        payments.iter().map(|payment| payment.2).sum::<i64>(),
        1_500,
        "the captures add up to the order total"
    );
    assert!(
        payments
            .iter()
            .all(|payment| payment.1.as_deref() == Some(order_id.as_str())
                && payment.3 == "captured"),
        "every payment belongs to the order and is captured: {payments:?}"
    );

    // The split session is a real payment session, captured with the rest.
    let split_status: String =
        sqlx::query_scalar("SELECT status FROM payment_sessions WHERE id = ?")
            .bind(&split_session_id)
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
    assert_eq!(split_status, "captured");

    // The collection now names the order it paid for.
    let collection_order: Option<String> =
        sqlx::query_scalar("SELECT order_id FROM payment_collections")
            .fetch_one(fixture.db.pool())
            .await
            .unwrap();
    assert_eq!(
        collection_order.as_deref(),
        Some(order_id.as_str()),
        "the collection of the checkout is the money of the order"
    );

    // And the storefront reads back the settled collection with both sessions.
    let (status, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{collection}");
    assert_eq!(collection["payment_collection"]["order_id"], order_id);
    assert_eq!(collection["payment_collection"]["status"], "captured");
    assert_eq!(fixture.session_amounts(&collection), vec![1_100, 400]);
}

#[tokio::test]
async fn a_split_beyond_the_checkout_session_is_rejected() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;

    // The split comes out of the checkout's own session, so asking for more
    // than that session holds cannot be covered.
    for amount in [1_500, 2_000] {
        let (status, body) = fixture
            .split_session(
                &checkout_id,
                json!({"provider": "manual", "amount": amount}),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    let (_, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(
        fixture.session_amounts(&collection),
        vec![1_500],
        "a rejected split changes nothing"
    );

    // The checkout is still payable on its own session.
    let (status, completed) = fixture.complete(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["order"]["total"], 1_500);
    assert_eq!(recorded_payments(&fixture).await.len(), 1);
}

#[tokio::test]
async fn a_split_in_another_currency_is_rejected() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;

    let (status, body) = fixture
        .split_session(
            &checkout_id,
            json!({"provider": "manual", "amount": 400, "currency_code": "EUR"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (_, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(fixture.session_amounts(&collection), vec![1_500]);
}

#[tokio::test]
async fn a_split_requires_a_checkout_that_is_paying() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;

    let (status, canceled, _) = send(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/checkouts/{checkout_id}/cancel"),
        None,
        &[("host", HOST)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["checkout"]["status"], "canceled");

    let (status, body) = fixture
        .split_session(&checkout_id, json!({"provider": "manual", "amount": 400}))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn a_split_needs_a_positive_amount_of_a_known_provider() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;

    for amount in [0, -400] {
        let (status, body) = fixture
            .split_session(
                &checkout_id,
                json!({"provider": "manual", "amount": amount}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }

    let (status, body) = fixture
        .split_session(&checkout_id, json!({"provider": "nope", "amount": 400}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (_, collection) = fixture.get_collection(&checkout_id).await;
    assert_eq!(fixture.session_amounts(&collection), vec![1_500]);
}

#[tokio::test]
async fn a_checkout_without_a_collection_is_not_found_instead_of_broken() {
    let fixture = fixture(1_500).await;
    let (_, checkout_id) = fixture.open_checkout().await;
    // Checkouts that pre-date payment collections have no collection row; the
    // storefront must report that plainly and the completion must still work.
    sqlx::query("DELETE FROM payment_collections")
        .execute(fixture.db.pool())
        .await
        .unwrap();

    let (status, body) = fixture.get_collection(&checkout_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = fixture
        .split_session(&checkout_id, json!({"provider": "manual", "amount": 400}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = fixture.get_collection("chk_unknown").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, completed) = fixture.complete(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["order"]["total"], 1_500);
    assert_eq!(recorded_payments(&fixture).await.len(), 1);
}
