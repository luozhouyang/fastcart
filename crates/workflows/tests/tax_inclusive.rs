//! Stores that list tax-inclusive prices must not have tax added on top.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_module_tax::{CreateTaxRateInput, CreateTaxRegionInput, TaxService};
use fastcart_workflows::{CartContext, CartWorkflow};

#[tokio::test]
async fn inclusive_pricing_extracts_tax_instead_of_adding_it() {
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

    let context = CartContext::new(CountryCode::new("DE").unwrap()).with_inclusive_pricing(true);
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
    let cart = carts
        .set_shipping_method(&cart.id, &option.id, &context)
        .await
        .unwrap();

    // 1000 + 500 are gross: the tax is inside them, not on top.
    assert!(cart.tax_lines.iter().all(|line| line.is_tax_inclusive));
    assert_eq!(cart.totals.tax_total, 136);
    assert_eq!(cart.totals.total, 1_500);
}
