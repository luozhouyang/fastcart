use super::support::*;
use super::*;

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
    assert_eq!(products["count"], 1);
    assert_eq!(products["products"][0]["id"], linked.id);
    assert_eq!(
        products["products"][0]["sales_channel_ids"],
        serde_json::json!([channel.id])
    );
    assert!(products["products"][0].get("calculated_price").is_none());

    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/products?currency_code=USD",
        None,
        &key_headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    let calculated = &products["products"][0]["calculated_price"];
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
    assert_eq!(products["products"][0]["calculated_price"]["amount"], 2_500);

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
    assert_eq!(product["product"]["calculated_price"]["amount"], 2_500);

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
    assert_eq!(products["count"], 2);
}

#[tokio::test]
async fn admin_manages_product_types_and_assigns_them_to_products() {
    let fixture = fixture().await;
    let (store_id, admin_auth, csrf) =
        bootstrap_store_with_admin(&fixture, "types.example.com").await;
    let store_id_header = store_id.to_string();
    let headers = [
        ("cookie", admin_auth.as_str()),
        ("x-csrf-token", csrf.as_str()),
        ("x-store-id", store_id_header.as_str()),
    ];

    let (status, product_type) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/catalog/product-types",
        Some(serde_json::json!({"value": "Shirt"})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{product_type}");
    let product_type_id = product_type["id"].as_str().unwrap().to_owned();
    assert!(product_type_id.starts_with("ptype_"));

    let (status, duplicate) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/catalog/product-types",
        Some(serde_json::json!({"value": "shirt"})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");

    let (status, product) = request(
        &fixture.app,
        "POST",
        "/api/v1/admin/catalog/products",
        Some(serde_json::json!({
            "title": "Typed Hoodie",
            "product_type_id": product_type_id.as_str(),
            "variants": [{"title": "Default", "sku": "TYPED-1"}]
        })),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{product}");
    let product_id = product["id"].as_str().unwrap().to_owned();
    assert_eq!(product["product_type_id"], product_type_id.as_str());

    let (status, products) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/products",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{products}");
    assert_eq!(products["pagination"]["total"], 1);
    assert_eq!(
        products["data"][0]["product_type_id"],
        product_type_id.as_str()
    );

    let (status, product_types) = request(
        &fixture.app,
        "GET",
        "/api/v1/admin/catalog/product-types",
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product_types}");
    assert_eq!(product_types["pagination"]["total"], 1);
    assert_eq!(product_types["data"][0]["value"], "Shirt");
    assert_eq!(product_types["data"][0]["id"], product_type_id.as_str());

    let (status, updated) = request(
        &fixture.app,
        "PATCH",
        &format!("/api/v1/admin/catalog/product-types/{product_type_id}"),
        Some(serde_json::json!({"value": "T-Shirt"})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["value"], "T-Shirt");
    assert_eq!(updated["id"], product_type_id.as_str());

    let (status, product) = request(
        &fixture.app,
        "PATCH",
        &format!("/api/v1/admin/catalog/products/{product_id}"),
        Some(serde_json::json!({"product_type_id": ""})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product}");
    assert!(product["product_type_id"].is_null());

    let (status, product) = request(
        &fixture.app,
        "PATCH",
        &format!("/api/v1/admin/catalog/products/{product_id}"),
        Some(serde_json::json!({"product_type_id": product_type_id.as_str()})),
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product}");
    assert_eq!(product["product_type_id"], product_type_id.as_str());

    let (status, body) = request(
        &fixture.app,
        "DELETE",
        &format!("/api/v1/admin/catalog/product-types/{product_type_id}"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let (status, product) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/catalog/products/{product_id}"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{product}");
    assert!(product["product_type_id"].is_null());

    let (status, body) = request(
        &fixture.app,
        "GET",
        &format!("/api/v1/admin/catalog/product-types/{product_type_id}"),
        None,
        &headers,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn storefront_context_reports_store_identity_and_currency() {
    let fixture = fixture().await;
    let _store_id = bootstrap_store(&fixture, "context.example.com").await;

    let (status, body) = request(
        &fixture.app,
        "GET",
        "/api/v1/store/context",
        None,
        &[("host", "context.example.com")],
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["domain"], "context.example.com");
    assert_eq!(body["locale"], "en-US");
    assert_eq!(body["currency_code"], "USD");
    assert!(body["store_id"].as_str().is_some_and(|id| !id.is_empty()));
}
