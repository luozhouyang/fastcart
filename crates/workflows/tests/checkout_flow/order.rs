use super::support::*;
use super::*;

/// The order must record exactly the money that was charged. A cart-level
/// discount changes what the customer pays, so an order that ignores it stores
/// a total nobody was ever billed — and refunds are capped by that total.
#[tokio::test]
async fn placed_order_records_the_cart_total_that_was_charged() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Mug".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                sku: Some("MUG-1".to_owned()),
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
        .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 1_000))
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
            name: "Standard shipping".to_owned(),
            amount_minor: 500,
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
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.clone()),
            1,
            &context,
        )
        .await
        .unwrap();
    let cart = carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();
    // A cart-level discount, as an order-level promotion would apply it.
    let cart = carts
        .cart_service()
        .add_discount(
            &cart.id,
            fastcart_module_cart::DiscountInput {
                promotion_id: None,
                code: Some("SAVE300".to_owned()),
                amount: 300,
                is_automatic: false,
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();
    assert_eq!(cart.totals.discount_total, 300);
    assert_eq!(cart.totals.total, 1_200);

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let checkout_workflow = CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );
    let checkout = checkout_workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    let completed = checkout_workflow
        .complete(&checkout.id, &context)
        .await
        .unwrap();

    assert_eq!(
        completed.payment.amount, 1_200,
        "the customer paid the cart total"
    );
    assert_eq!(
        completed.order.total, 1_200,
        "the order must record the amount that was charged"
    );
    assert_eq!(completed.order.discount_total, 300);
}
