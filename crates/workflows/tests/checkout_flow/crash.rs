use super::support::*;
use super::*;

#[tokio::test]
async fn completion_recovers_after_crash_between_capture_and_confirmation() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    // Minimal catalog, price and shipping setup; inventory is not managed here.
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Crash test product".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(false),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_500))
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
    let cart = carts
        .add_item(&cart.id, &VariantId::from_string(variant_id), 1, &context)
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
    // A second service sharing the same provider registry lets the test capture
    // the payment directly, simulating a crash right after provider capture.
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
    let payment_session = checkout.payment_session_id.clone().unwrap();
    let captured = capture_payments
        .capture(payment_session.as_str())
        .await
        .unwrap();
    assert_eq!(captured.status, PaymentState::Captured);

    // The runner crashed after capture, before `payment_confirmed`, leaving a
    // claim whose lease has long expired.
    sqlx::query(
        "UPDATE checkout_sessions
         SET placement_token = 'crashed-runner', placement_claimed_at = ?
         WHERE id = ?",
    )
    .bind(now_timestamp() - 600)
    .bind(checkout.id.as_str())
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(
        workflow.get(&checkout.id).await.unwrap().status,
        CheckoutStatus::PaymentPending
    );

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.checkout.status, CheckoutStatus::OrderPlaced);
    assert_eq!(completed.payment.status, PaymentState::Captured);
    assert_eq!(
        completed.order.payment_status,
        fastcart_module_order::PaymentStatus::Captured
    );

    let token: Option<String> =
        sqlx::query_scalar("SELECT placement_token FROM checkout_sessions WHERE id = ?")
            .bind(checkout.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(token.is_none(), "placement claim should be cleared");

    // A replay returns the placed result without capturing or ordering again.
    let replay = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(replay.order.id, completed.order.id);
    assert_eq!(replay.payment.id, completed.payment.id);
}
