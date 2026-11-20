//! An order-target promotion is a promotion like any other.
//!
//! Its discount is recorded as a cart-level row instead of a line adjustment,
//! and the checkout claims promotions from the adjustments it can see. Claiming
//! only the line and shipping adjustments left order-level promotions
//! unaccounted for: a usage limit, per-customer limit or campaign budget never
//! bound them, so the same discount could be redeemed without end.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Metadata, ShippingOptionId, StoreId, VariantId,
};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CreatePromotionInput, PromotionService, PromotionTarget, PromotionType,
};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

/// A store selling one 1 000 variant with a free shipping option.
async fn store(db: &StoreDatabase, currency: &CurrencyCode) -> (String, ShippingOptionId) {
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Mug".to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
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
            name: "Free".to_owned(),
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

    (variant_id, option.id.clone())
}

/// A cart holding one unit of `variant_id`, priced and ready to check out.
async fn cart_with_item(
    carts: &CartWorkflow,
    variant_id: &str,
    option_id: &ShippingOptionId,
    currency: &CurrencyCode,
    context: &CartContext,
) -> Cart {
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
            &VariantId::from_string(variant_id.to_owned()),
            1,
            context,
        )
        .await
        .unwrap();
    // The returned snapshot is the recalculated one: prices, totals and the
    // promotions that apply are only settled by the pass this call runs.
    carts
        .set_shipping_method(&cart.id, option_id, context)
        .await
        .unwrap()
}

fn checkout_workflow(db: &StoreDatabase) -> CheckoutWorkflow {
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    )
}

/// Places one checkout for a fresh cart and returns the order, or the error.
async fn place(
    db: &StoreDatabase,
    variant_id: &str,
    option_id: &ShippingOptionId,
    currency: &CurrencyCode,
    context: &CartContext,
) -> Result<fastcart_module_order::Order, AppError> {
    let carts = CartWorkflow::new(db.clone());
    let cart = cart_with_item(&carts, variant_id, option_id, currency, context).await;
    let checkout = checkout_workflow(db);
    let created = checkout
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            context.clone(),
        ))
        .await?;
    Ok(checkout.complete(&created.id, context).await?.order)
}

#[tokio::test]
async fn completing_a_checkout_claims_one_use_of_an_order_level_promotion() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();
    let (variant_id, option_id) = store(&db, &currency).await;

    let promotions = PromotionService::new(db.clone());
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Order discount", PromotionType::FixedAmount, 200)
                .with_target(PromotionTarget::Order)
                .active()
                .automatic()
                .with_usage_limit(5),
        )
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let order = place(&db, &variant_id, &option_id, &currency, &context)
        .await
        .unwrap();

    assert_eq!(
        order.discount_total, 200,
        "the order-level promotion must discount the order"
    );
    assert_eq!(
        promotions.usage_count(&promotion.id).await.unwrap(),
        1,
        "completing a checkout spends one use of an order-level promotion"
    );
}

#[tokio::test]
async fn an_exhausted_order_level_promotion_rejects_the_next_checkout() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();
    let (variant_id, option_id) = store(&db, &currency).await;

    let promotions = PromotionService::new(db.clone());
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Order discount", PromotionType::FixedAmount, 200)
                .with_target(PromotionTarget::Order)
                .active()
                .automatic()
                .with_usage_limit(1),
        )
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let checkout = checkout_workflow(&db);

    // Both carts are quoted while the single use is still unclaimed, so each of
    // them carries the discount when it reaches completion. Only the claim
    // decides which one keeps it.
    let mut checkouts = Vec::new();
    for _ in 0..2 {
        let cart = cart_with_item(&carts, &variant_id, &option_id, &currency, &context).await;
        assert!(
            cart.totals.discount_total > 0,
            "the order-level discount is quoted while the use is available"
        );
        checkouts.push(
            checkout
                .create(CreateCheckoutRequest::new(
                    cart.id.clone(),
                    "manual",
                    context.clone(),
                ))
                .await
                .unwrap(),
        );
    }

    checkout
        .complete(&checkouts[0].id, &context)
        .await
        .expect("the first checkout claims the single use");

    // The second cart cannot keep a discount whose one use is spent. It is
    // rejected — by the re-pricing guard here, since the discount is gone from
    // the cart the moment the use is gone — and never becomes an order.
    let error = checkout
        .complete(&checkouts[1].id, &context)
        .await
        .expect_err("the second checkout must not keep a discount it cannot claim");
    assert!(
        matches!(&error, AppError::Conflict(_)),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        promotions.usage_count(&promotion.id).await.unwrap(),
        1,
        "the rejected checkout must not add a use"
    );
    let orders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(orders, 1, "only the first checkout places an order");
}
