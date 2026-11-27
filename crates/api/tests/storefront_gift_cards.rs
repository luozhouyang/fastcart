//! Storefront HTTP coverage of gift cards as a payment method.
//!
//! A gift card is only useful when a buyer can apply it to a cart and have it
//! pay for the order, so this test drives that path over real HTTP: issue a
//! card through the payment service, apply it through the storefront API,
//! complete the checkout, and check the money that moved and the ledger that
//! records it.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{CurrencyCode, StoreId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_payment::{CreateGiftCardInput, GiftCardRedemptionStatus, PaymentService};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const HOST: &str = "shop.example.com";

/// A store with one published product priced at `amount`, a free shipping
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
    /// Issues a card and returns its code.
    async fn issue_card(&self, amount_minor: i64) -> String {
        PaymentService::new(self.db.clone())
            .issue_gift_card(CreateGiftCardInput::new("usd", amount_minor))
            .await
            .unwrap()
            .code
    }

    fn payment(&self) -> PaymentService {
        PaymentService::new(self.db.clone())
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

    async fn apply_gift_card(&self, cart_id: &str, code: &str) -> (StatusCode, Value) {
        let (status, body, _) = send(
            &self.app,
            "POST",
            &format!("/api/v1/store/carts/{cart_id}/gift-cards"),
            Some(json!({ "code": code })),
            &[("host", HOST)],
        )
        .await;
        (status, body)
    }

    /// Opens a checkout for the cart with the manual provider.
    async fn open_checkout(&self, cart_id: &str) -> (StatusCode, Value) {
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
async fn a_gift_card_pays_the_cart_and_is_redeemed_at_checkout() {
    let fixture = fixture(5_000).await;
    let code = fixture.issue_card(2_000).await;
    let cart_id = fixture.open_cart().await;

    let (status, cart) = fixture.apply_gift_card(&cart_id, &code).await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    let applied = &cart["cart"]["gift_cards"][0];
    assert_eq!(applied["amount"], 2_000);
    assert_eq!(applied["balance"], 2_000);
    assert_eq!(applied["applied_amount"], 2_000);
    assert!(
        applied["masked_code"]
            .as_str()
            .unwrap()
            .ends_with(&code[12..]),
        "the masked code keeps only the last four characters: {applied}"
    );
    assert!(
        !cart.to_string().contains(&code),
        "the full code must not leak"
    );
    assert_eq!(cart["cart"]["totals"]["gift_card_total"], 2_000);
    assert_eq!(cart["cart"]["totals"]["total"], 3_000);

    let (status, checkout) = fixture.open_checkout(&cart_id).await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    assert_eq!(checkout["checkout"]["totals"]["gift_card_total"], 2_000);
    let checkout_id = checkout["checkout"]["id"].as_str().unwrap().to_owned();

    let (status, completed) = fixture.complete(&checkout_id).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    // The order records the money that did not reach the provider: the total
    // is what the card did not pay.
    assert_eq!(completed["order"]["total"], 3_000);
    let order_id = completed["order"]["id"].as_str().unwrap().to_owned();

    let card = fixture
        .payment()
        .find_gift_card_by_code(&code)
        .await
        .unwrap();
    assert_eq!(card.balance_minor, 0, "the card paid its full balance");

    let redemptions = fixture
        .payment()
        .gift_card_redemptions_for_card(&card.id)
        .await
        .unwrap();
    assert_eq!(redemptions.len(), 1);
    assert_eq!(redemptions[0].amount_minor, 2_000);
    assert_eq!(redemptions[0].status, GiftCardRedemptionStatus::Redeemed);
    assert_eq!(
        redemptions[0].order_id.as_deref(),
        Some(order_id.as_str()),
        "the ledger names the order the card paid for"
    );
}

#[tokio::test]
async fn a_card_spent_somewhere_else_rejects_the_completion() {
    let fixture = fixture(5_000).await;
    let code = fixture.issue_card(2_000).await;
    let cart_id = fixture.open_cart().await;

    let (status, cart) = fixture.apply_gift_card(&cart_id, &code).await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    let (status, checkout) = fixture.open_checkout(&cart_id).await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    let checkout_id = checkout["checkout"]["id"].as_str().unwrap().to_owned();

    // The customer spends part of the card elsewhere after the checkout quotes
    // it, leaving less than the amount the checkout was created with.
    fixture
        .payment()
        .redeem_gift_card(&code, 1_500)
        .await
        .unwrap();

    let (status, rejected) = fixture.complete(&checkout_id).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a card that no longer covers its quote must not be spent: {rejected}"
    );

    let (status, checkout, _) = send(
        &fixture.app,
        "GET",
        &format!("/api/v1/store/checkouts/{checkout_id}"),
        None,
        &[("host", HOST)],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        checkout["checkout"]["status"], "order_placed",
        "the rejected completion must not place an order"
    );

    let card = fixture
        .payment()
        .find_gift_card_by_code(&code)
        .await
        .unwrap();
    assert_eq!(
        card.balance_minor, 500,
        "the completion spent nothing beyond the direct redemption"
    );
}
