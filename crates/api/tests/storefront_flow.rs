//! End-to-end storefront HTTP flow: catalog, cart, checkout and order.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fastcart_api::{ApiConfig, AppContext, build_router};
use fastcart_infrastructure::{DatabaseOptions, StoreCache, StoreDatabaseManager};
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use fastcart_module_pricing::{CreatePriceInput, CreatePriceListInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_module_store::{
    CreatePublishableApiKeyInput, CreateRegionInput, CreateSalesChannelInput, RegionService,
    StoreService,
};
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_platform::PlatformStore;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Fixture {
    app: Router,
    context: Arc<AppContext>,
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
    Fixture {
        app: build_router(context.clone()),
        context,
        _dir: dir,
    }
}

async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

fn cookie_value(cookies: &[String], name: &str) -> Option<String> {
    cookies.iter().find_map(|cookie| {
        let (cookie_name, rest) = cookie.split_once('=')?;
        (cookie_name.trim() == name).then(|| rest.split(';').next().unwrap_or("").to_owned())
    })
}

async fn bootstrap_store(fixture: &Fixture, domain: &str) -> StoreId {
    let setup = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/platform/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "owner@example.com",
                        "password": "hunter2hunter2"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::OK);
    let cookies: Vec<String> = setup
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");

    let (status, store) = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": domain
        })),
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    StoreId::from_string(store["id"].as_str().unwrap())
}

#[tokio::test]
async fn storefront_products_respect_publishable_key_channels_and_calculated_prices() {
    let fixture = fixture().await;
    let store_id = bootstrap_store(&fixture, "shop.example.com").await;
    let db = fixture.context.databases.database(&store_id).await.unwrap();

    let channel = StoreService::new(db.clone())
        .create_sales_channel(CreateSalesChannelInput {
            name: "Web".to_owned(),
            description: None,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let key = RegionService::new(db.clone())
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: "Web storefront".to_owned(),
            sales_channel_ids: vec![channel.id.clone()],
        })
        .await
        .unwrap();
    let region = RegionService::new(db.clone())
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            tax_region_id: None,
            payment_providers: Vec::new(),
            is_default: true,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();

    let catalog = CatalogService::new(db.clone());
    let linked = catalog
        .create_product(CreateProductInput {
            title: "Linked Hoodie".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("LINKED-L".to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let unlinked = catalog
        .create_product(CreateProductInput {
            title: "Unlinked Hoodie".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("UNLINKED-L".to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&linked.id).await.unwrap();
    catalog.publish_product(&unlinked.id).await.unwrap();
    catalog
        .set_product_sales_channels(&linked.id, std::slice::from_ref(&channel.id))
        .await
        .unwrap();

    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(
            &linked.variants[0].id,
            CurrencyCode::new("USD").unwrap(),
            3_000,
        ))
        .await
        .unwrap();
    let sale = PricingService::new(db.clone())
        .create_price_list(CreatePriceListInput::new("Summer sale").active())
        .await
        .unwrap();
    PricingService::new(db.clone())
        .create_price(
            CreatePriceInput::new(
                &linked.variants[0].id,
                CurrencyCode::new("USD").unwrap(),
                2_500,
            )
            .with_price_list(&sale.id),
        )
        .await
        .unwrap();

    let key_headers = [
        ("host", "shop.example.com"),
        ("x-publishable-api-key", key.token.as_str()),
    ];

    // The key restricts the listing to linked products and calculated prices
    // are only included when a currency is resolvable.
    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 1);
    assert_eq!(products["data"][0]["id"], linked.id);
    assert_eq!(
        products["data"][0]["sales_channel_ids"],
        serde_json::json!([channel.id])
    );
    assert!(products["data"][0].get("calculated_price").is_none());

    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products?currency_code=USD",
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    let calculated = &products["data"][0]["calculated_price"];
    assert_eq!(calculated["amount"], 2_500);
    assert_eq!(calculated["currency_code"], "USD");
    assert_eq!(calculated["price_list_id"], sale.id);
    assert_eq!(calculated["price_list_type"], "sale");
    assert_eq!(calculated["original_amount"], 3_000);

    let (status, products) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/store/products?region_id={}", region.id),
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["data"][0]["calculated_price"]["amount"], 2_500);

    // Products outside the key's channels are not addressable.
    let (status, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products/unlinked-hoodie",
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, product) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products/linked-hoodie?currency_code=USD&quantity=1",
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product}");
    assert_eq!(product["calculated_price"]["amount"], 2_500);

    // Without a key the channel filter is not applied.
    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "shop.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 2);
}

#[tokio::test]
async fn storefront_cart_checkout_and_order_flow() {
    let fixture = fixture().await;

    // Bootstrap an administrator and a store.
    let setup = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/platform/auth/setup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": "owner@example.com",
                        "password": "hunter2hunter2"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(setup.status(), StatusCode::OK);
    let cookies: Vec<String> = setup
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let session = cookie_value(&cookies, "fc_admin_session").unwrap();
    let csrf = cookie_value(&cookies, "fc_csrf").unwrap();
    let admin_auth = format!("fc_admin_session={session}; fc_csrf={csrf}");
    let admin_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
    ];

    let (status, store) = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Demo",
            "slug": "demo",
            "primary_domain": "shop.example.com"
        })),
        &admin_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{store}");
    assert_eq!(store["status"], "active");
    let store_id = StoreId::from_string(store["id"].as_str().unwrap());

    // Seed the store database through module services.
    let db = fixture.context.databases.database(&store_id).await.unwrap();
    let currency = CurrencyCode::new("USD").unwrap();
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Hoodie".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("HOODIE-L".to_owned()),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 3_000))
        .await
        .unwrap();
    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: Some("HOODIE-L".to_owned()),
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 5, "restock", None)
        .await
        .unwrap();
    let tax = TaxService::new(db.clone());
    let region = tax
        .create_region(CreateTaxRegionInput {
            name: "United States".to_owned(),
            country_code: CountryCode::new("US").unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: region.id.clone(),
        tax_category_id: None,
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
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
            currency: currency.clone(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap();

    let host = [("host", "shop.example.com")];

    // Published catalog is visible.
    let (status, products) =
        request(&fixture.app, "GET", "/api/v1/store/products", None, &host).await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 1);
    assert_eq!(products["data"][0]["title"], "Hoodie");

    let (status, product_response) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products/hoodie",
        None,
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product_response}");

    let (status, availability) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products/hoodie/availability",
        None,
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{availability}");
    assert_eq!(availability[0]["available"], 5);

    let (status, shipping_options) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/shipping/options?country_code=US&subtotal=3000",
        None,
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{shipping_options}");
    assert_eq!(shipping_options.as_array().unwrap().len(), 1);

    // Anonymous cart.
    let (status, cart) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({"currency_code": "USD"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cart}");
    let cart_id = cart["id"].as_str().unwrap().to_owned();

    let (status, cart) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/carts/{cart_id}/items"),
        Some(serde_json::json!({
            "variant_id": variant_id,
            "quantity": 1,
            "country_code": "US"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    assert_eq!(cart["totals"]["item_subtotal"], 3_000);

    let (status, cart) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/carts/{cart_id}/shipping-method"),
        Some(serde_json::json!({
            "shipping_option_id": option.id.to_string(),
            "country_code": "US"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    assert_eq!(cart["totals"]["tax_total"], 300);
    assert_eq!(cart["totals"]["total"], 3_300);

    let (status, checkout) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/checkouts",
        Some(serde_json::json!({
            "cart_id": cart_id,
            "email": "buyer@example.com",
            "payment_provider": "manual",
            "country_code": "US"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{checkout}");
    let checkout_id = checkout["id"].as_str().unwrap().to_owned();

    let (status, completed) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/checkouts/{checkout_id}/complete"),
        Some(serde_json::json!({"country_code": "US"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["order"]["total"], 3_300);
    assert_eq!(completed["order"]["status"], "confirmed");
    assert_eq!(completed["order"]["payment_status"], "captured");
    let order_id = completed["order"]["id"].as_str().unwrap().to_owned();

    // Idempotent completion.
    let (status, replay) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/checkouts/{checkout_id}/complete"),
        Some(serde_json::json!({"country_code": "US"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["order"]["id"], order_id);

    // Stock committed.
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id.clone()))
            .await
            .unwrap(),
        4
    );

    // Cancelling the order returns committed stock and is idempotent.
    let admin_store_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
        ("x-store-id", store_id.as_str()),
    ];
    let (status, canceled) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/cancel"),
        Some(serde_json::json!({"reason": "customer request"})),
        &admin_store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["status"], "canceled");
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id.clone()))
            .await
            .unwrap(),
        5
    );

    let (status, replay_cancel) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/admin/orders/{order_id}/cancel"),
        Some(serde_json::json!({})),
        &admin_store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay_cancel}");
    assert_eq!(replay_cancel["status"], "canceled");
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id))
            .await
            .unwrap(),
        5
    );

    // Unknown hosts never resolve to a store.
    let (status, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products",
        None,
        &[("host", "unknown.example.com")],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Cross-store isolation: an admin request for a different store fails.
    let (status, _) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &[
            ("cookie", admin_auth.as_str()),
            ("x-csrf-token", csrf.as_str()),
            ("x-store-id", "store_missing"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner also belongs to a second store, but that store's database
    // cannot see the first store's catalog or orders.
    let (status, second_store) = request(
        &fixture.app,
        "POST",
        "/api/v1/platform/stores",
        Some(serde_json::json!({
            "name": "Second",
            "slug": "second",
            "primary_domain": "second.example.com"
        })),
        &admin_headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{second_store}");
    let second_store_id = second_store["id"].as_str().unwrap();
    let second_store_headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
        ("x-store-id", second_store_id),
    ];

    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &second_store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 0);

    let (status, orders) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/orders",
        None,
        &second_store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{orders}");
    assert_eq!(orders["pagination"]["total"], 0);

    let (status, _) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/orders/{order_id}"),
        None,
        &second_store_headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
