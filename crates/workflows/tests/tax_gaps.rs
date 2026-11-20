//! A destination the store taxes must not end up silently untaxed.
//!
//! "No tax region covers this destination" is a legitimate configuration — the
//! store does not collect tax there. "A region covers it but no rate applies to
//! this line" is a misconfiguration, and treating both the same way means an
//! item is quietly sold tax-free with nothing in the data to show why.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_tax::{CreateTaxRegionInput, TaxService};
use fastcart_workflows::{CartContext, CartWorkflow};

async fn cart_for(country: &str, configure_region: bool) -> fastcart_module_cart::Cart {
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

    if configure_region {
        // A region that governs the destination but carries no rates at all.
        TaxService::new(db.clone())
            .create_region(CreateTaxRegionInput {
                name: "Germany".to_owned(),
                country_code: CountryCode::new(country).unwrap(),
                province_code: None,
                metadata: None,
            })
            .await
            .unwrap();
    }

    let context = CartContext::new(CountryCode::new(country).unwrap());
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
        .add_item(&cart.id, &VariantId::from_string(variant_id), 1, &context)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_destination_with_no_tax_region_records_no_tax() {
    let cart = cart_for("US", false).await;

    assert_eq!(cart.totals.total, 1_000);
    assert!(
        cart.tax_lines.is_empty(),
        "a store that does not tax this destination needs no tax line: {:?}",
        cart.tax_lines
    );
}

#[tokio::test]
async fn a_region_without_a_matching_rate_is_visible_on_the_cart() {
    let cart = cart_for("DE", true).await;

    assert_eq!(
        cart.totals.total, 1_000,
        "an absent rate is still no tax, and must not invent one"
    );
    assert_eq!(cart.tax_lines.len(), 1);
    let line = &cart.tax_lines[0];
    assert_eq!(line.rate_bps, 0);
    assert_eq!(line.amount, 0);
    assert!(
        line.name.contains("Germany"),
        "the line must name the region whose configuration is incomplete: {line:?}"
    );
}
