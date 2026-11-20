//! What `CartWorkflow::recalculate` selects for each line must not change.
//!
//! Quantity tiers, customer groups, sales channels, validity windows and a
//! closed window all still decide a line's price one line at a time. The
//! batching that makes the pass cheap is measured in `cart_pricing_batching`,
//! which lives in its own test binary: counting statements process-wide needs
//! no other test issuing pricing queries at the same time.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_workflows::{CartContext, CartWorkflow};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn context() -> CartContext {
    CartContext::new(CountryCode::new("US").unwrap())
}

/// Seeds the customer group and sales channel prices may be scoped to.
async fn seed_scope_rows(db: &StoreDatabase) {
    let now = fastcart_kernel::now_timestamp();
    for statement in [
        "INSERT INTO customer_groups (id, name, metadata, created_at, updated_at) \
         VALUES ('grp_vip', 'VIP', '{}', ?, ?)",
        "INSERT INTO sales_channels (id, name, is_disabled, metadata, created_at, updated_at) \
         VALUES ('chan_web', 'Web', 0, '{}', ?, ?)",
    ] {
        sqlx::query(statement)
            .bind(now)
            .bind(now)
            .execute(db.pool())
            .await
            .unwrap();
    }
}

async fn create_cart(workflow: &CartWorkflow, sales_channel_id: Option<String>) -> Cart {
    workflow
        .create_cart(CreateCartInput {
            currency_code: usd().to_string(),
            customer_id: None,
            email: Some("buyer@example.com".to_owned()),
            sales_channel_id,
            region_id: None,
            metadata: Metadata::empty(),
            expires_in_seconds: None,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_line_whose_price_window_has_passed_keeps_the_missing_price_conflict() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
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

    let now = fastcart_kernel::now_timestamp();
    PricingService::new(db.clone())
        .create_price(
            CreatePriceInput::new(&variant_id, usd(), 1_000).with_window(None, Some(now + 60)),
        )
        .await
        .unwrap();

    let workflow = CartWorkflow::new(db.clone());
    let context = context();
    let cart = create_cart(&workflow, None).await;
    workflow
        .add_item(
            &cart.id,
            &VariantId::from_string(variant_id.clone()),
            1,
            &context,
        )
        .await
        .unwrap();

    // Recalculating once the only price window has closed must fail the whole
    // recalculation with the same conflict the per-line lookup raised.
    let mut after_the_window = context.clone();
    after_the_window.at = now + 3_600;
    let error = workflow
        .recalculate(&cart.id, &after_the_window)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, fastcart_kernel::AppError::Conflict(message)
            if message.contains("no price is configured for variant")
                && message.contains(&variant_id)
                && message.contains("USD")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn recalculate_keeps_tiered_channel_and_group_prices_per_line() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    seed_scope_rows(&db).await;
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: "Mug".to_owned(),
            variants: (0..3)
                .map(|index| CreateVariantInput {
                    title: format!("Variant {index}"),
                    sku: Some(format!("MUG-{index}")),
                    manage_inventory: Some(false),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let tiered = product.variants[0].id.clone();
    let channelled = product.variants[1].id.clone();
    let grouped = product.variants[2].id.clone();

    let pricing = PricingService::new(db.clone());
    // A quantity tier, a channel price and a group price that must each beat
    // the plain base price of their own variant.
    pricing
        .create_price(CreatePriceInput::new(&tiered, usd(), 1_000))
        .await
        .unwrap();
    pricing
        .create_price(CreatePriceInput::new(&tiered, usd(), 800).with_quantity_range(10, None))
        .await
        .unwrap();
    pricing
        .create_price(CreatePriceInput::new(&channelled, usd(), 1_000))
        .await
        .unwrap();
    pricing
        .create_price(CreatePriceInput::new(&channelled, usd(), 500).with_sales_channel("chan_web"))
        .await
        .unwrap();
    pricing
        .create_price(CreatePriceInput::new(&grouped, usd(), 1_000))
        .await
        .unwrap();
    pricing
        .create_price(CreatePriceInput::new(&grouped, usd(), 700).with_customer_group("grp_vip"))
        .await
        .unwrap();

    let workflow = CartWorkflow::new(db.clone());
    let context = context().with_customer_groups(vec!["grp_vip".to_owned()]);
    let cart = create_cart(&workflow, Some("chan_web".to_owned())).await;
    for (variant, quantity) in [(&tiered, 10), (&channelled, 1), (&grouped, 1)] {
        workflow
            .add_item(
                &cart.id,
                &VariantId::from_string(variant.clone()),
                quantity,
                &context,
            )
            .await
            .unwrap();
    }

    let cart = workflow.recalculate(&cart.id, &context).await.unwrap();
    let unit_prices: Vec<i64> = cart.items.iter().map(|item| item.unit_price).collect();
    assert_eq!(
        unit_prices,
        vec![800, 500, 700],
        "each line must keep the price its own quantity and scope select"
    );
}
