//! An order-target promotion discounts the whole order, shipping included.
//!
//! The calculator books its amount on two legs: what the item value can absorb,
//! and what is left over for shipping. The cart caps a cart-level row at the
//! item subtotal, so a discount larger than the items only reached the customer
//! in part — the rest was recorded in a row the totals then clamped away.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, ShippingOptionId, StoreId, VariantId};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CreatePromotionInput, PromotionService, PromotionTarget, PromotionType,
};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

#[tokio::test]
async fn an_order_discount_beyond_the_item_value_discounts_shipping() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = usd();

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
            name: "Standard".to_owned(),
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

    // 1 200 off an order whose items are worth 1 000: the remaining 200 has to
    // come off the shipping, or the customer is charged for money the promotion
    // promised to cover.
    PromotionService::new(db.clone())
        .create_promotion(
            CreatePromotionInput::new("Order discount", PromotionType::FixedAmount, 1_200)
                .with_target(PromotionTarget::Order)
                .active()
                .automatic(),
        )
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
    let cart = carts
        .set_shipping_method(
            &cart.id,
            &ShippingOptionId::from_string(option.id.to_string()),
            &context,
        )
        .await
        .unwrap();

    assert_eq!(
        cart.totals.discount_total, 1_000,
        "the item value absorbs the first leg of the order discount"
    );
    assert_eq!(
        cart.totals.shipping_discount_total, 200,
        "the rest of the order discount comes off the shipping"
    );
    assert_eq!(
        cart.totals.total, 300,
        "the customer pays what is left after the whole discount"
    );
}
