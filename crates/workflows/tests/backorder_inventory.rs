//! End-to-end inventory accounting for variants that allow backorders.
//!
//! A backorder variant must still reserve and commit stock: units sold while
//! nothing is on hand stay visible as negative stock until they are restocked,
//! and strict variants keep refusing to oversell in the same batch. A backorder
//! variant with no inventory item at all has nothing to record against, so its
//! line is skipped and the checkout passes as before.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CountryCode, CurrencyCode, InventoryItemId, InventoryLocationId, Metadata, ShippingOptionId,
    StoreId, VariantId,
};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_checkout::CheckoutStatus;
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryLevel, InventoryService,
};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

const DEFAULT_LOCATION_ID: &str = "loc_default";

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

fn context() -> CartContext {
    CartContext::new(CountryCode::new("US").unwrap())
}

fn workflow(db: &StoreDatabase) -> CheckoutWorkflow {
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    CheckoutWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    )
}

/// Creates a published product with one inventory-managed variant and a price.
async fn variant_product(db: &StoreDatabase, title: &str, allow_backorder: bool) -> String {
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![CreateVariantInput {
                title: "Default".to_owned(),
                manage_inventory: Some(true),
                allow_backorder: Some(allow_backorder),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    catalog.publish_product(&product.id).await.unwrap();
    let variant_id = product.variants[0].id.clone();
    PricingService::new(db.clone())
        .create_price(CreatePriceInput::new(&variant_id, usd(), 1_000))
        .await
        .unwrap();
    variant_id
}

/// Creates the inventory item behind `variant_id` with `stocked` units at a
/// fresh, non-default location.
async fn stocked_item(
    inventory: &InventoryService,
    variant_id: &str,
    stocked: i64,
) -> (InventoryItemId, InventoryLocationId) {
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
            sku: None,
            variant_id: Some(VariantId::from_string(variant_id)),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, stocked, "restock", None)
        .await
        .unwrap();
    (item.id, location.id)
}

async fn shipping_option(db: &StoreDatabase) -> ShippingOptionId {
    let shipping = ShippingService::new(db.clone());
    let profile = shipping
        .create_profile(CreateShippingProfileInput {
            name: "Default".to_owned(),
            profile_type: None,
            metadata: None,
        })
        .await
        .unwrap();
    shipping
        .create_option(CreateShippingOptionInput {
            profile_id: profile.id.clone(),
            name: "Standard shipping".to_owned(),
            amount_minor: 0,
            currency: usd(),
            provider_id: None,
            provider_data: None,
            fulfillment_set_id: None,
            is_enabled: Some(true),
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

/// Creates a cart holding one line per `(variant, quantity)` pair.
async fn cart_with_items(
    db: &StoreDatabase,
    items: &[(String, i64)],
    context: &CartContext,
) -> fastcart_kernel::CartId {
    let carts = CartWorkflow::new(db.clone());
    let cart = carts
        .create_cart(CreateCartInput {
            currency_code: usd().to_string(),
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
        .set_shipping_method(&cart.id, &shipping_option(db).await, context)
        .await
        .unwrap();
    for (variant_id, quantity) in items {
        carts
            .add_item(
                &cart.id,
                &VariantId::from_string(variant_id.clone()),
                *quantity,
                context,
            )
            .await
            .unwrap();
    }
    cart.id
}

/// The `(checkout id, location, quantity, status, line_item_id)` rows of every
/// reservation in the store, ordered by creation.
async fn reservation_rows(
    db: &StoreDatabase,
) -> Vec<(String, String, i64, String, Option<String>)> {
    sqlx::query_as(
        "SELECT checkout_id, location_id, quantity, status, line_item_id \
         FROM inventory_reservations ORDER BY created_at, rowid",
    )
    .fetch_all(db.pool())
    .await
    .unwrap()
}

/// Whether the default location row was created.
async fn default_location_count(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM inventory_locations WHERE id = ?")
        .bind(DEFAULT_LOCATION_ID)
        .fetch_one(db.pool())
        .await
        .unwrap()
}

async fn level(
    inventory: &InventoryService,
    item: &InventoryItemId,
    location: &str,
) -> InventoryLevel {
    inventory
        .get_level(item, &InventoryLocationId::from_string(location))
        .await
        .unwrap()
}

/// A backordered sale reserves its shortfall, commits it as negative stock and
/// can be restocked back to zero.
#[tokio::test]
async fn a_backordered_checkout_commits_negative_stock_and_can_be_restocked() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let inventory = InventoryService::new(db.clone());
    let variant = variant_product(&db, "Backordered", true).await;
    let (item, location) = stocked_item(&inventory, &variant, 0).await;
    let context = context();
    let cart = cart_with_items(&db, &[(variant.clone(), 3)], &context).await;
    let workflow = workflow(&db);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();

    // The three units nothing can cover are booked on the default location.
    let rows = reservation_rows(&db).await;
    assert_eq!(rows.len(), 1, "the backordered line must be reserved");
    assert_eq!(rows[0].0, checkout.id.as_str());
    assert_eq!(rows[0].1, DEFAULT_LOCATION_ID);
    assert_eq!(rows[0].2, 3);
    assert_eq!(rows[0].3, "reserved");
    let default_location = level(&inventory, &item, DEFAULT_LOCATION_ID).await;
    assert_eq!(default_location.stocked_quantity, 0);
    assert_eq!(default_location.reserved_quantity, 3);
    assert_eq!(
        inventory
            .available_quantity(
                &item,
                &InventoryLocationId::from_string(DEFAULT_LOCATION_ID)
            )
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        inventory
            .available_quantity(&item, &location)
            .await
            .unwrap(),
        0
    );

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.order.items.len(), 1);
    assert_eq!(completed.order.items[0].quantity, 3);

    // Committing the backorder leaves the sold units visible as negative stock
    // instead of silently dropping them.
    let default_location = level(&inventory, &item, DEFAULT_LOCATION_ID).await;
    assert_eq!(default_location.stocked_quantity, -3);
    assert_eq!(default_location.reserved_quantity, 0);
    let rows = reservation_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].3, "committed");
    assert_eq!(
        rows[0].4.as_deref(),
        Some(completed.order.items[0].id.as_str()),
        "the committed reservation must point at its order line"
    );
    assert_eq!(
        inventory
            .variant_availability(&VariantId::from_string(variant.clone()))
            .await
            .unwrap(),
        0,
        "availability must never go negative"
    );
    let committed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_movements WHERE reason = 'commit' AND quantity_delta = -3",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(committed, 1, "the sale must be recorded as a movement");

    // Restocking the sold units pulls the level back to zero.
    inventory
        .adjust_stock(
            &item,
            &InventoryLocationId::from_string(DEFAULT_LOCATION_ID),
            3,
            "restock",
            None,
        )
        .await
        .unwrap();
    let default_location = level(&inventory, &item, DEFAULT_LOCATION_ID).await;
    assert_eq!(default_location.stocked_quantity, 0);
    assert_eq!(default_location.reserved_quantity, 0);
    assert_eq!(
        inventory
            .available_quantity(
                &item,
                &InventoryLocationId::from_string(DEFAULT_LOCATION_ID)
            )
            .await
            .unwrap(),
        0
    );
}

/// A cart mixing a strict variant that is short with a backorder variant fails
/// as a whole: the strict line still cannot oversell.
#[tokio::test]
async fn a_strict_line_in_a_mixed_cart_still_refuses_to_oversell() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let inventory = InventoryService::new(db.clone());
    let strict_variant = variant_product(&db, "Strict", false).await;
    let backorder_variant = variant_product(&db, "Backordered", true).await;
    let (strict_item, strict_location) = stocked_item(&inventory, &strict_variant, 1).await;
    let (backorder_item, backorder_location) =
        stocked_item(&inventory, &backorder_variant, 0).await;
    let context = context();
    // The backordered line comes first so a batch that books its shortfall
    // before validating the strict line is caught.
    let cart = cart_with_items(
        &db,
        &[(backorder_variant.clone(), 1), (strict_variant.clone(), 2)],
        &context,
    )
    .await;
    let workflow = workflow(&db);

    let error = workflow
        .create(CreateCheckoutRequest::new(cart, "manual", context))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("insufficient stock"),
        "unexpected error: {error}"
    );
    assert!(
        error.to_string().contains(strict_variant.as_str()),
        "the conflict must name the short variant: {error}"
    );

    // Nothing of the rejected batch survives: no reservation, no backorder
    // level, no reserved quantity.
    assert_eq!(reservation_rows(&db).await.len(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM inventory_levels WHERE location_id = ?",
        )
        .bind(DEFAULT_LOCATION_ID)
        .fetch_one(db.pool())
        .await
        .unwrap(),
        0
    );
    let strict_level = level(&inventory, &strict_item, strict_location.as_str()).await;
    assert_eq!(strict_level.stocked_quantity, 1);
    assert_eq!(strict_level.reserved_quantity, 0);
    let backorder_level = level(&inventory, &backorder_item, backorder_location.as_str()).await;
    assert_eq!(backorder_level.stocked_quantity, 0);
    assert_eq!(backorder_level.reserved_quantity, 0);
}

/// The `(location, quantity, status)` of the reservation bound to one order
/// line.
async fn reservation_of_order_line(
    db: &StoreDatabase,
    order_item_id: &str,
) -> (String, i64, String) {
    sqlx::query_as(
        "SELECT location_id, quantity, status FROM inventory_reservations WHERE line_item_id = ?",
    )
    .bind(order_item_id)
    .fetch_one(db.pool())
    .await
    .unwrap()
}

/// A cart mixing a stocked line with a backordered one places both, and every
/// reservation stays bound to its own order line and location.
#[tokio::test]
async fn a_mixed_cart_books_each_line_against_its_own_stock() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let inventory = InventoryService::new(db.clone());
    let strict_variant = variant_product(&db, "Strict", false).await;
    let backorder_variant = variant_product(&db, "Backordered", true).await;
    let (strict_item, strict_location) = stocked_item(&inventory, &strict_variant, 2).await;
    let (backorder_item, _) = stocked_item(&inventory, &backorder_variant, 0).await;
    let context = context();
    let cart = cart_with_items(
        &db,
        &[(strict_variant.clone(), 2), (backorder_variant.clone(), 1)],
        &context,
    )
    .await;
    let workflow = workflow(&db);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(cart, "manual", context.clone()))
        .await
        .unwrap();
    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.order.items.len(), 2);
    assert_eq!(reservation_rows(&db).await.len(), 2);

    for order_item in &completed.order.items {
        let variant = order_item.variant_id.clone().unwrap();
        let (location, quantity, status) =
            reservation_of_order_line(&db, order_item.id.as_str()).await;
        assert_eq!(status, "committed");
        assert_eq!(quantity, order_item.quantity);
        if variant.as_str() == strict_variant {
            assert_eq!(location, strict_location.as_str());
        } else {
            assert_eq!(
                location, DEFAULT_LOCATION_ID,
                "the backordered line is owed from the default location"
            );
        }
    }

    let strict_level = level(&inventory, &strict_item, strict_location.as_str()).await;
    assert_eq!(strict_level.stocked_quantity, 0);
    assert_eq!(strict_level.reserved_quantity, 0);
    assert_eq!(
        level(&inventory, &backorder_item, DEFAULT_LOCATION_ID)
            .await
            .stocked_quantity,
        -1
    );
}

/// A backorder variant whose variant has no inventory item has no ledger to
/// record the line against. Checkout must pass silently — no reservation and
/// no error — exactly as it did before backorder lines were reserved.
#[tokio::test]
async fn a_backorder_variant_without_an_inventory_item_checks_out_without_a_reservation() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let variant = variant_product(&db, "Backordered", true).await;
    let context = context();
    let cart = cart_with_items(&db, &[(variant, 2)], &context).await;
    let workflow = workflow(&db);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(cart, "manual", context.clone()))
        .await
        .expect("a backorder variant without an inventory item must not fail checkout");
    assert_eq!(checkout.status, CheckoutStatus::PaymentPending);
    assert_eq!(
        reservation_rows(&db).await.len(),
        0,
        "there is no inventory item, so there is nothing to reserve"
    );
    assert_eq!(
        default_location_count(&db).await,
        0,
        "the skipped line must not create the default location"
    );

    // Completion does not touch inventory either and must place the order.
    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.order.items.len(), 1);
    assert_eq!(completed.order.items[0].quantity, 2);
    assert_eq!(reservation_rows(&db).await.len(), 0);
}

/// A backorder variant that *does* have an inventory item keeps recording its
/// shortfall as negative stock.
#[tokio::test]
async fn a_backorder_variant_with_an_inventory_item_still_books_its_shortfall() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let inventory = InventoryService::new(db.clone());
    let variant = variant_product(&db, "Backordered", true).await;
    let (item, _) = stocked_item(&inventory, &variant, 0).await;
    let context = context();
    let cart = cart_with_items(&db, &[(variant, 2)], &context).await;
    let workflow = workflow(&db);

    let checkout = workflow
        .create(CreateCheckoutRequest::new(cart, "manual", context.clone()))
        .await
        .unwrap();
    let rows = reservation_rows(&db).await;
    assert_eq!(rows.len(), 1, "the shortfall must still be reserved");
    assert_eq!(rows[0].1, DEFAULT_LOCATION_ID);
    assert_eq!(rows[0].2, 2);
    assert_eq!(
        level(&inventory, &item, DEFAULT_LOCATION_ID)
            .await
            .reserved_quantity,
        2
    );

    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    assert_eq!(completed.order.items.len(), 1);
    assert_eq!(
        level(&inventory, &item, DEFAULT_LOCATION_ID)
            .await
            .stocked_quantity,
        -2,
        "the sold backorder units stay visible as negative stock"
    );
}

/// A strict variant with no inventory item can never be satisfied and must
/// keep failing the checkout with the historical error.
#[tokio::test]
async fn a_strict_variant_without_an_inventory_item_still_refuses_checkout() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let variant = variant_product(&db, "Strict", false).await;
    let context = context();
    let cart = cart_with_items(&db, &[(variant.clone(), 1)], &context).await;

    let error = workflow(&db)
        .create(CreateCheckoutRequest::new(cart, "manual", context))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("insufficient stock for variant {variant}")),
        "unexpected error: {error}"
    );
    assert_eq!(reservation_rows(&db).await.len(), 0);
}
