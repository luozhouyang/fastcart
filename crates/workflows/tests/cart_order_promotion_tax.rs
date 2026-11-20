//! An order-target promotion is part of the price, so it is part of the base.
//!
//! Line discounts have always reduced the amount a line is taxed on. A
//! cart-level discount did not: it is recorded on the cart rather than on the
//! line, and the tax pass only ever looked at the line's own adjustments. A
//! customer who took an order-level discount paid tax on money the promotion
//! had already taken off the bill.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CreatePromotionInput, PromotionService, PromotionTarget, PromotionType,
};
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_workflows::{CartContext, CartWorkflow};

#[tokio::test]
async fn an_order_discount_shrinks_the_amount_that_is_taxed() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let currency = CurrencyCode::new("USD").unwrap();

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

    // 10 % over the counter, charged on top of the price.
    let tax = TaxService::new(db.clone());
    let region = tax
        .create_region(CreateTaxRegionInput {
            name: "Germany".to_owned(),
            country_code: CountryCode::new("DE").unwrap(),
            province_code: None,
            metadata: None,
        })
        .await
        .unwrap();
    tax.create_rate(CreateTaxRateInput {
        tax_region_id: region.id.clone(),
        tax_category_id: None,
        name: "VAT".to_owned(),
        rate_bps: 1_000,
        is_inclusive: false,
        is_default: true,
        metadata: None,
    })
    .await
    .unwrap();

    PromotionService::new(db.clone())
        .create_promotion(
            CreatePromotionInput::new("Order discount", PromotionType::FixedAmount, 200)
                .with_target(PromotionTarget::Order)
                .active()
                .automatic(),
        )
        .await
        .unwrap();

    let context = CartContext::new(CountryCode::new("DE").unwrap());
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

    assert_eq!(cart.totals.discount_total, 200);
    assert_eq!(
        cart.totals.tax_total, 80,
        "tax is charged on the discounted price, not the list price"
    );
    assert_eq!(
        cart.totals.total, 880,
        "the customer pays the discounted price plus tax on it"
    );
}
