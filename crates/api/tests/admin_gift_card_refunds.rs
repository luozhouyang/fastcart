//! HTTP coverage of refunds that restore gift card balances.
//!
//! A checkout can be paid partly by gift card, leaving the payment provider
//! with only the rest. Refunding that payment must put the card money back on
//! the card first and send only the uncovered remainder to the provider. This
//! test drives the whole path over HTTP: issue a card through the admin API,
//! spend it through the storefront checkout, refund through the admin payments
//! API and read the split back from the response and the card.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
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

/// A store with one published product, free shipping, an owner session and the
/// HTTP surface of the running application.
struct Fixture {
    app: Router,
    store_id: StoreId,
    owner_auth: String,
    csrf: String,
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
    let owner_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
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
            ("cookie", owner_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    let store_id = StoreId::from_string(store["id"].as_str().unwrap());

    // Seed the store database through module services; everything a buyer or
    // an administrator does goes over HTTP.
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
        store_id,
        owner_auth,
        csrf,
        variant_id,
        shipping_option_id: option.id.to_string(),
        _context: context,
        _dir: dir,
    }
}

impl Fixture {
    /// Sends one request as the store owner.
    async fn admin(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, value, _) = send(
            &self.app,
            method,
            uri,
            body,
            &[
                ("cookie", self.owner_auth.as_str()),
                ("x-csrf-token", self.csrf.as_str()),
                ("x-store-id", self.store_id.as_str()),
            ],
        )
        .await;
        (status, value)
    }

    /// Issues a card through the admin API and returns the card body.
    async fn issue_card(&self, amount_minor: i64) -> Value {
        let (status, card) = self
            .admin(
                "POST",
                "/api/v1/admin/gift-cards",
                Some(json!({"currency_code": "USD", "amount_minor": amount_minor})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{card}");
        card
    }

    /// Opens a cart with the seeded variant and a free shipping method.
    async fn open_cart(&self) -> String {
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
        cart_id
    }

    async fn apply_gift_card(&self, cart_id: &str, code: &str) {
        let (status, body, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/carts/{cart_id}/gift-cards"),
            Some(json!({ "code": code })),
            &[("host", HOST)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["cart"]["totals"]["gift_card_total"], 2_000, "{body}");
    }

    /// Opens a checkout for the cart with the manual provider.
    async fn open_checkout(&self, cart_id: &str) -> String {
        let (status, body, _) = send(
            &self.app,
            "POST",
            "/api/v1/store/checkouts",
            Some(json!({
                "cart_id": cart_id,
                "email": "buyer@example.com",
                "payment_provider": "manual",
                "country_code": "US"
            })),
            &[("host", HOST)],
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["checkout"]["id"].as_str().unwrap().to_owned()
    }

    async fn complete(&self, checkout_id: &str) -> Value {
        let (status, body, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/checkouts/{checkout_id}/complete"),
            Some(json!({"country_code": "US"})),
            &[("host", HOST)],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
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
        let (key, value) = cookie.split_once('=')?;
        (key == name).then(|| value.split(';').next().unwrap_or_default().to_owned())
    })
}

#[tokio::test]
async fn admin_refunds_restore_the_gift_card_before_the_provider() {
    let fixture = fixture(5_000).await;
    let card = fixture.issue_card(2_000).await;
    let card_id = card["id"].as_str().unwrap().to_owned();
    let code = card["code"].as_str().unwrap().to_owned();

    // The customer pays 2_000 with the card and 3_000 through the provider.
    let cart_id = fixture.open_cart().await;
    fixture.apply_gift_card(&cart_id, &code).await;
    let checkout_id = fixture.open_checkout(&cart_id).await;
    let completed = fixture.complete(&checkout_id).await;
    assert_eq!(completed["order"]["total"], 3_000, "{completed}");
    let order_id = completed["order"]["id"].as_str().unwrap().to_owned();

    let (status, page) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/payments?order_id={order_id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["pagination"]["total"], 1, "{page}");
    let payment = &page["data"][0];
    assert_eq!(payment["amount"], 3_000, "{payment}");
    assert_eq!(payment["status"], "captured", "{payment}");
    let payment_id = payment["id"].as_str().unwrap().to_owned();

    let (status, card) = fixture
        .admin("GET", &format!("/api/v1/admin/gift-cards/{card_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["balance_minor"], 0, "{card}");

    // The first refund is entirely covered by the card redemption: the card is
    // restored and the provider is not called at all.
    let (status, first) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/payments/{payment_id}/refunds"),
            Some(json!({
                "amount": 1_500,
                "idempotency_key": "return-refund:http-1"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["amount"], 1_500, "{first}");
    assert_eq!(first["status"], "succeeded", "{first}");
    assert_eq!(first["gift_card_refunded"], 1_500, "{first}");
    assert_eq!(first["provider_refunded"], 0, "{first}");
    assert_eq!(first["provider_refund_id"], Value::Null, "{first}");

    let (status, card) = fixture
        .admin("GET", &format!("/api/v1/admin/gift-cards/{card_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["balance_minor"], 1_500, "{card}");

    // The second refund finds 500 of the redemption still uncredited and sends
    // the remaining 1_000 to the provider.
    let (status, second) = fixture
        .admin(
            "POST",
            &format!("/api/v1/admin/payments/{payment_id}/refunds"),
            Some(json!({
                "amount": 1_500,
                "idempotency_key": "return-refund:http-2"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_eq!(second["amount"], 1_500, "{second}");
    assert_eq!(second["gift_card_refunded"], 500, "{second}");
    assert_eq!(second["provider_refunded"], 1_000, "{second}");
    assert!(
        second["provider_refund_id"].is_string(),
        "a provider remainder carries its refund reference: {second}"
    );

    let (status, card) = fixture
        .admin("GET", &format!("/api/v1/admin/gift-cards/{card_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(
        card["balance_minor"], 2_000,
        "the card's spent balance is fully restored: {card}"
    );

    // The refund list exposes the same split for both refunds.
    let (status, refunds) = fixture
        .admin(
            "GET",
            &format!("/api/v1/admin/payments/{payment_id}/refunds"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{refunds}");
    assert_eq!(refunds.as_array().unwrap().len(), 2, "{refunds}");
    assert_eq!(refunds[0]["gift_card_refunded"], 1_500, "{refunds}");
    assert_eq!(refunds[0]["provider_refunded"], 0, "{refunds}");
    assert_eq!(refunds[1]["gift_card_refunded"], 500, "{refunds}");
    assert_eq!(refunds[1]["provider_refunded"], 1_000, "{refunds}");

    let (status, payment) = fixture
        .admin("GET", &format!("/api/v1/admin/payments/{payment_id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{payment}");
    assert_eq!(payment["status"], "refunded", "{payment}");
}
