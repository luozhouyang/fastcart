//! Reservations must stay traceable to the order line (and location) that
//! consumed them once a checkout becomes an order.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CountryCode, CurrencyCode, InventoryItemId, InventoryLocationId, Metadata, ReturnId,
    ShippingOptionId, StoreId, VariantId,
};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, ReturnRestockLine,
};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
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

/// Creates a published product with one inventory-managed variant.
async fn variant_product(db: &StoreDatabase, title: &str) -> String {
    let catalog = CatalogService::new(db.clone());
    let product = catalog
        .create_product(CreateProductInput {
            title: title.to_owned(),
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
        .create_price(CreatePriceInput::new(&variant_id, usd(), 1_000))
        .await
        .unwrap();
    variant_id
}

/// Creates the inventory item behind `variant_id` with `stocked` units at its
/// own location.
async fn stocked_item(
    inventory: &InventoryService,
    variant_id: &str,
    stocked: i64,
) -> (InventoryItemId, InventoryLocationId) {
    let location = inventory
        .create_location(CreateLocationInput {
            name: format!("Warehouse {variant_id}"),
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

/// The number of reservations that bind `order_item_id` to the stock of
/// `variant_id` at `location_id` for the placed order.
async fn bindings_for_order_item(
    db: &StoreDatabase,
    order_item_id: &str,
    order_id: &str,
    variant_id: &str,
    quantity: i64,
    location_id: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations r \
         JOIN inventory_items i ON i.id = r.inventory_item_id \
         WHERE r.line_item_id = ? AND r.order_id = ? AND r.status = 'committed' \
           AND i.variant_id = ? AND r.quantity = ? AND r.location_id = ?",
    )
    .bind(order_item_id)
    .bind(order_id)
    .bind(variant_id)
    .bind(quantity)
    .bind(location_id)
    .fetch_one(db.pool())
    .await
    .unwrap()
}

/// A shop with a placed two-line order, ready for traceability assertions.
struct PlacedOrder {
    workflow: CheckoutWorkflow,
    inventory: InventoryService,
    carts: CartWorkflow,
    context: CartContext,
    cart_id: fastcart_kernel::CartId,
    checkout_id: fastcart_kernel::CheckoutId,
    order: fastcart_module_order::Order,
}

/// Places an order for two variants, each stocked at its own location.
async fn place_two_line_order(db: &StoreDatabase) -> PlacedOrder {
    let inventory = InventoryService::new(db.clone());
    let first_variant = variant_product(db, "First").await;
    let second_variant = variant_product(db, "Second").await;
    stocked_item(&inventory, &first_variant, 5).await;
    stocked_item(&inventory, &second_variant, 5).await;
    let context = CartContext::new(CountryCode::new("US").unwrap());
    let carts = CartWorkflow::new(db.clone());
    let cart = cart_with_items(db, &[(first_variant, 2), (second_variant, 1)], &context).await;
    let workflow = workflow(db);
    let checkout = workflow
        .create(CreateCheckoutRequest::new(
            cart.clone(),
            "manual",
            context.clone(),
        ))
        .await
        .unwrap();
    let completed = workflow.complete(&checkout.id, &context).await.unwrap();
    PlacedOrder {
        workflow,
        inventory,
        carts,
        context,
        cart_id: cart,
        checkout_id: checkout.id,
        order: completed.order,
    }
}

/// The stock location of every variant's inventory item.
async fn location_of_variant(db: &StoreDatabase, variant_id: &str) -> String {
    sqlx::query_scalar(
        "SELECT r.location_id FROM inventory_reservations r \
         JOIN inventory_items i ON i.id = r.inventory_item_id \
         WHERE i.variant_id = ?",
    )
    .bind(variant_id)
    .fetch_one(db.pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn a_placed_order_traces_each_reservation_to_its_order_line() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let placed = place_two_line_order(&db).await;
    let order = placed.order;
    let cart = placed.carts.get_cart(&placed.cart_id).await.unwrap();

    let cart_lines: Vec<String> = cart
        .items
        .iter()
        .map(|item| item.id.as_str().to_owned())
        .collect();
    assert_eq!(order.items.len(), 2);

    for order_item in &order.items {
        let variant_id = order_item.variant_id.clone().unwrap();
        let location = location_of_variant(&db, variant_id.as_str()).await;
        assert_eq!(
            bindings_for_order_item(
                &db,
                order_item.id.as_str(),
                order.id.as_str(),
                variant_id.as_str(),
                order_item.quantity,
                &location,
            )
            .await,
            1,
            "order line {} must own the reservation of its variant",
            order_item.id
        );
    }

    // The cart line identifiers are gone: a reservation now points at the order
    // line that consumed it.
    let stale: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE line_item_id = ? OR line_item_id = ?",
    )
    .bind(&cart_lines[0])
    .bind(&cart_lines[1])
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(stale, 0, "no reservation may keep the cart line identifier");
}

#[tokio::test]
async fn replaying_completion_keeps_the_order_line_bindings() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let placed = place_two_line_order(&db).await;
    let order = placed.order;

    // A replay returns the same order without touching the reservations.
    let replay = placed
        .workflow
        .complete(&placed.checkout_id, &placed.context)
        .await
        .unwrap();
    assert_eq!(replay.order.id, order.id);

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE status = 'committed'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(total, 2, "no reservation may be duplicated or lost");

    for order_item in &order.items {
        let variant_id = order_item.variant_id.clone().unwrap();
        let location = location_of_variant(&db, variant_id.as_str()).await;
        assert_eq!(
            bindings_for_order_item(
                &db,
                order_item.id.as_str(),
                order.id.as_str(),
                variant_id.as_str(),
                order_item.quantity,
                &location,
            )
            .await,
            1
        );
    }
}

#[tokio::test]
async fn a_return_restock_still_lands_on_the_committed_reservations_location() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let placed = place_two_line_order(&db).await;
    let inventory = placed.inventory;
    let order = placed.order;

    let order_item = &order.items[0];
    let variant_id = order_item.variant_id.clone().unwrap();
    let item_id: String = sqlx::query_scalar("SELECT id FROM inventory_items WHERE variant_id = ?")
        .bind(variant_id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
    let item_id = InventoryItemId::from_string(item_id);
    let location: String = sqlx::query_scalar(
        "SELECT location_id FROM inventory_reservations WHERE order_id = ? AND inventory_item_id = ?",
    )
    .bind(order.id.as_str())
    .bind(item_id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();

    let restocked = inventory
        .restock_return(
            &order.id,
            &ReturnId::new(),
            &[ReturnRestockLine::new(
                order_item.id.clone(),
                variant_id,
                order_item.quantity,
                order_item.quantity,
            )],
        )
        .await
        .unwrap();
    assert_eq!(restocked, order_item.quantity);
    let level = inventory
        .get_level(
            &item_id,
            &InventoryLocationId::from_string(location.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        level.stocked_quantity, 5,
        "the return must restock the location the order committed from"
    );
}
