//! Rebinding reservations from cart lines to the order lines they became.
//!
//! A reservation is recorded while a cart still owns the line, but the order
//! assigns fresh line identifiers when it is placed. Rebinding keeps the
//! reservation traceable to the order line (and the location it was taken
//! from) without touching anything else.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, OrderId, StoreId, VariantId,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, InventoryService, Reference, ReservationLine, ReservationRebinding,
};

async fn setup() -> (StoreDatabase, InventoryService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = InventoryService::new(db.clone());
    (db, service)
}

/// Creates the default location, an item backing `variant` and `quantity` units
/// of stock on it.
async fn stocked_item(
    db: &StoreDatabase,
    service: &InventoryService,
    variant: &VariantId,
    quantity: i64,
) -> InventoryItemId {
    let now = fastcart_kernel::now_timestamp();
    sqlx::query(
        "INSERT OR IGNORE INTO inventory_locations \
         (id, name, address, metadata, created_at, updated_at) \
         VALUES ('loc_default', 'Default', '{}', '{}', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
    let item = service
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap()
        .id;
    service
        .adjust_stock(
            &item,
            &InventoryLocationId::from_string("loc_default"),
            quantity,
            "receiving",
            None,
        )
        .await
        .unwrap();
    item
}

fn line(variant: &VariantId, quantity: i64, line_item_id: &str) -> ReservationLine {
    ReservationLine {
        variant_id: variant.clone(),
        quantity,
        line_item_id: Some(LineItemId::from_string(line_item_id)),
    }
}

#[tokio::test]
async fn rebinding_points_reservations_at_their_order_lines() {
    let (db, service) = setup().await;
    let first_variant = VariantId::new();
    let second_variant = VariantId::new();
    let _first_item = stocked_item(&db, &service, &first_variant, 5).await;
    let _second_item = stocked_item(&db, &service, &second_variant, 5).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve(
            vec![
                line(&first_variant, 2, "cart_line_1"),
                line(&second_variant, 1, "cart_line_2"),
            ],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();

    let rebound = service
        .rebind_reservations(
            &checkout,
            &order,
            &[
                ReservationRebinding::new(
                    LineItemId::from_string("cart_line_1"),
                    LineItemId::from_string("order_item_1"),
                ),
                ReservationRebinding::new(
                    LineItemId::from_string("cart_line_2"),
                    LineItemId::from_string("order_item_2"),
                ),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rebound, 2);

    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .line_item_id,
        Some(LineItemId::from_string("order_item_1"))
    );
    assert_eq!(
        service
            .get_reservation(&reservations[1].id)
            .await
            .unwrap()
            .line_item_id,
        Some(LineItemId::from_string("order_item_2"))
    );

    // Rebinding is a rename, not a stock change.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_reservations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        2
    );

    // Replaying the same mapping writes nothing.
    assert_eq!(
        service
            .rebind_reservations(
                &checkout,
                &order,
                &[ReservationRebinding::new(
                    LineItemId::from_string("cart_line_1"),
                    LineItemId::from_string("order_item_1"),
                )],
            )
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .line_item_id,
        Some(LineItemId::from_string("order_item_1"))
    );
}

#[tokio::test]
async fn rebinding_only_touches_the_checkout_it_is_given() {
    let (db, service) = setup().await;
    let variant = VariantId::new();
    let _item = stocked_item(&db, &service, &variant, 10).await;

    let first_checkout = CheckoutId::new();
    let second_checkout = CheckoutId::new();
    let first = service
        .reserve(
            vec![line(&variant, 1, "cart_line_1")],
            Reference::new(first_checkout.clone()),
            60,
        )
        .await
        .unwrap();
    let second = service
        .reserve(
            vec![line(&variant, 1, "cart_line_1")],
            Reference::new(second_checkout.clone()),
            60,
        )
        .await
        .unwrap();

    let rebound = service
        .rebind_reservations(
            &first_checkout,
            &OrderId::new(),
            &[ReservationRebinding::new(
                LineItemId::from_string("cart_line_1"),
                LineItemId::from_string("order_item_1"),
            )],
        )
        .await
        .unwrap();
    assert_eq!(rebound, 1);
    assert_eq!(
        service
            .get_reservation(&first[0].id)
            .await
            .unwrap()
            .line_item_id,
        Some(LineItemId::from_string("order_item_1"))
    );
    assert_eq!(
        service
            .get_reservation(&second[0].id)
            .await
            .unwrap()
            .line_item_id,
        Some(LineItemId::from_string("cart_line_1")),
        "another checkout's reservation must not move"
    );
}

#[tokio::test]
async fn rebinding_converges_after_the_reservations_were_committed() {
    let (db, service) = setup().await;
    let variant = VariantId::new();
    let item = stocked_item(&db, &service, &variant, 3).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve(
            vec![line(&variant, 2, "cart_line_1")],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();

    // A runner that committed but crashed before rebinding still converges:
    // the committed reservation belongs to the order and may be rebound.
    let rebound = service
        .rebind_reservations(
            &checkout,
            &order,
            &[ReservationRebinding::new(
                LineItemId::from_string("cart_line_1"),
                LineItemId::from_string("order_item_1"),
            )],
        )
        .await
        .unwrap();
    assert_eq!(rebound, 1);
    let reservation = service.get_reservation(&reservations[0].id).await.unwrap();
    assert_eq!(
        reservation.line_item_id,
        Some(LineItemId::from_string("order_item_1"))
    );
    assert_eq!(reservation.order_id, Some(order));
    assert_eq!(
        service
            .available_quantity(&item, &InventoryLocationId::from_string("loc_default"))
            .await
            .unwrap(),
        1,
        "rebinding must not move stock"
    );
}

#[tokio::test]
async fn rebinding_ignores_lines_without_a_reservation() {
    let (db, service) = setup().await;
    let variant = VariantId::new();
    let _item = stocked_item(&db, &service, &variant, 2).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![line(&variant, 1, "cart_line_1")],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();

    assert_eq!(
        service
            .rebind_reservations(
                &checkout,
                &order,
                &[ReservationRebinding::new(
                    LineItemId::from_string("cart_line_missing"),
                    LineItemId::from_string("order_item_9"),
                )],
            )
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .rebind_reservations(&checkout, &order, &[])
            .await
            .unwrap(),
        0
    );
}
