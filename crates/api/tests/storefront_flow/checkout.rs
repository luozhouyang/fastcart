use super::support::*;
use super::*;

#[tokio::test]
async fn fresh_store_completes_checkout_with_provisioned_shipping() {
    let fixture = fixture().await;
    let store_id = bootstrap_store(&fixture, "fresh.example.com").await;
    let db = fixture.context.databases.database(&store_id).await.unwrap();
    let host = [("host", "fresh.example.com")];

    // Provisioning seeds a free default shipping option; a merchant only needs
    // to add a product, stock and a price before the first real purchase.
    let (status, context) =
        request(&fixture.app, "GET", "/api/v1/store/context", None, &host).await;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(context["currency_code"], "USD");

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "First product".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some("FIRST-1".to_owned()),
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
        .create_price(CreatePriceInput::new(
            &variant_id,
            CurrencyCode::new("USD").unwrap(),
            1_000,
        ))
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
            sku: Some("FIRST-1".to_owned()),
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 3, "restock", None)
        .await
        .unwrap();

    let (status, cart) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({"currency_code": "USD"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cart}");
    let cart_id = cart["cart"]["id"].as_str().unwrap().to_owned();

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

    let (status, cart) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/carts/{cart_id}/email"),
        Some(serde_json::json!({"email": "buyer@example.com"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cart}");

    let (status, options) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/shipping/options?country_code=US&subtotal=1000",
        None,
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{options}");
    let seeded = options["shipping_options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["option"]["id"] == "ship_default")
        .expect("provisioned default shipping option");
    assert_eq!(seeded["amount_minor"], 0);

    let (status, cart) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/carts/{cart_id}/shipping-method"),
        Some(serde_json::json!({
            "shipping_option_id": "ship_default",
            "country_code": "US"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cart}");
    assert_eq!(cart["cart"]["totals"]["total"], 1_000);

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
    let checkout_id = checkout["checkout"]["id"].as_str().unwrap().to_owned();

    let (status, completed) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/checkouts/{checkout_id}/complete"),
        Some(serde_json::json!({"country_code": "US"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["order"]["status"], "confirmed");
    assert_eq!(completed["order"]["payment_status"], "captured");
    assert_eq!(completed["order"]["total"], 1_000);
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id))
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn regions_scope_carts_checkouts_and_payment_providers() {
    let fixture = fixture().await;
    let store_id = bootstrap_store(&fixture, "regions.example.com").await;
    let db = fixture.context.databases.database(&store_id).await.unwrap();
    let region = RegionService::new(db.clone())
        .create_region(CreateRegionInput {
            name: "United States".to_owned(),
            currency_code: "USD".to_owned(),
            countries: vec!["US".to_owned()],
            tax_region_id: None,
            payment_providers: vec!["manual".to_owned()],
            is_default: true,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let host = [("host", "regions.example.com")];

    // Storefront clients discover the default region from the context call.
    let (status, context) =
        request(&fixture.app, "GET", "/api/v1/store/context", None, &host).await;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(
        context["default_region_id"].as_str(),
        Some(region.id.as_str())
    );

    // An explicit region whose currency differs from the cart is rejected
    // rather than silently repricing the cart.
    let (status, error) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({
            "currency_code": "EUR",
            "region_id": region.id.as_str()
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");

    // A default region in a different currency is not attached, so legacy
    // currency-only carts for other currencies keep working.
    let (status, legacy_cart) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({"currency_code": "EUR"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{legacy_cart}");
    assert!(legacy_cart["cart"]["region_id"].is_null());

    // A matching default region is attached automatically.
    let (status, default_cart) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({"currency_code": "USD"})),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{default_cart}");
    assert_eq!(
        default_cart["cart"]["region_id"].as_str(),
        Some(region.id.as_str())
    );
    let blocked_cart_id = default_cart["cart"]["id"].as_str().unwrap().to_owned();

    let (status, cart) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/carts",
        Some(serde_json::json!({
            "currency_code": "USD",
            "region_id": region.id.as_str()
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{cart}");
    assert_eq!(cart["cart"]["region_id"].as_str(), Some(region.id.as_str()));
    assert_eq!(cart["cart"]["currency_code"], "USD");
    let cart_id = cart["cart"]["id"].as_str().unwrap().to_owned();

    // Catalog, price and shipping needed to open a checkout.
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Region Hoodie".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("REGION-L".to_owned()),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(
            &variant_id,
            CurrencyCode::new("USD").unwrap(),
            2_500,
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

    // The region's country list constrains the destination.
    let (status, error) = request(
        &fixture.app,
        "POST",
        &format!("/api/v1/store/carts/{blocked_cart_id}/items"),
        Some(serde_json::json!({
            "variant_id": variant_id,
            "quantity": 1,
            "country_code": "GB"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("US"),
        "{error}"
    );

    for target in [&cart_id, &blocked_cart_id] {
        let (status, cart) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/store/carts/{target}/items"),
            Some(serde_json::json!({
                "variant_id": variant_id,
                "quantity": 1,
                "country_code": "US"
            })),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cart}");
        let (status, cart) = request(
            &fixture.app,
            "POST",
            &format!("/api/v1/store/carts/{target}/shipping-method"),
            Some(serde_json::json!({
                "shipping_option_id": option.id.to_string(),
                "country_code": "US"
            })),
            &host,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cart}");
    }

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
    assert_eq!(
        checkout["checkout"]["region_id"].as_str(),
        Some(region.id.as_str())
    );

    // The region's allowed providers gate checkout creation.
    let (status, error) = request(
        &fixture.app,
        "POST",
        "/api/v1/store/checkouts",
        Some(serde_json::json!({
            "cart_id": blocked_cart_id,
            "email": "buyer@example.com",
            "payment_provider": "stripe",
            "country_code": "US"
        })),
        &host,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["error"]["code"], "VALIDATION_ERROR");
    let message = error["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("stripe"), "{message}");
    assert!(message.contains("manual"), "{message}");

    // Validation runs before a session row is created.
    assert!(
        fastcart_module_checkout::CheckoutService::new(db)
            .get_by_cart(&fastcart_kernel::CartId::from_string(blocked_cart_id))
            .await
            .unwrap()
            .is_none()
    );
}
