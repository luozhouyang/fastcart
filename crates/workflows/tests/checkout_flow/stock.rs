use super::support::*;
use super::*;

#[tokio::test]
async fn checkout_requires_stock_availability() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Limited".to_owned(),
            variants: vec![CreateVariantInput {
                title: "One".to_owned(),
                manage_inventory: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_000))
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
            sku: None,
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 1, "restock", None)
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: currency.to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap();
    carts
        .add_item(&cart.id, &VariantId::from_string(variant_id), 2, &context)
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
    carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(db.clone(), providers);
    let workflow = CheckoutWorkflow::new(db, payments);
    let error = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context,
        ))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("insufficient stock"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn expired_reservations_refund_and_fail_the_checkout() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Last unit".to_owned(),
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
    let variant_id = product.variants[0].id.clone();

    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 2_000))
        .await
        .unwrap();

    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main warehouse".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: Some("LAST-1".to_owned()),
            variant_id: Some(VariantId::from_string(variant_id.clone())),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, 5, "restock", None)
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

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: currency.to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id: None,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap();
    carts
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.clone()),
            1,
            &context,
        )
        .await
        .unwrap();
    carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let capture_payments = PaymentService::with_registry(db.clone(), providers.clone());
    let workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(checkout.status, CheckoutStatus::PaymentPending);
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant_id.clone()))
            .await
            .unwrap(),
        4
    );

    // The customer paid, but the reservation lease expired before placement.
    // Maintenance releases reservations independently of the checkout, which
    // is exactly the state that used to leave a captured payment attached to
    // a confirmed order with no stock movement and no refund.
    let payment_session = checkout.payment_session_id.clone().unwrap();
    let captured = capture_payments
        .capture(payment_session.as_str())
        .await
        .unwrap();
    assert_eq!(captured.status, PaymentState::Captured);
    inventory.release_checkout(&checkout.id).await.unwrap();

    let error = workflow.complete(&checkout.id, &context).await.unwrap_err();
    assert!(
        error.to_string().contains("reservation"),
        "unexpected error: {error}"
    );

    let failed = workflow.get(&checkout.id).await.unwrap();
    assert_eq!(failed.status, CheckoutStatus::Failed);
    assert!(failed.order_id.is_none());

    let order_status: String = sqlx::query_scalar("SELECT status FROM orders")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(order_status, "canceled");

    let payment_status: String = sqlx::query_scalar("SELECT status FROM payments LIMIT 1")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(payment_status, "refunded");

    let session_status: String =
        sqlx::query_scalar("SELECT status FROM payment_sessions WHERE id = ?")
            .bind(payment_session.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(session_status, "captured");

    let refunds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM refunds")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(refunds, 1, "compensation must refund exactly once");

    // The reservation was released before completion, so no commit movement
    // exists and stock stays untouched rather than being decremented twice.
    let (stocked, reserved): (i64, i64) = sqlx::query_as(
        "SELECT stocked_quantity, reserved_quantity FROM inventory_levels \
         WHERE inventory_item_id = ? AND location_id = ?",
    )
    .bind(item.id.as_str())
    .bind(location.id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!((stocked, reserved), (5, 0));

    // A replay of the failed checkout must not create a second order or a
    // second refund.
    assert!(workflow.complete(&checkout.id, &context).await.is_err());
    let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(orders, 1);
    let refunds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM refunds")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(refunds, 1);
}
