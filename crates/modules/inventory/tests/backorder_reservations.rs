//! Reservation behavior for lines that explicitly allow backorders.
//!
//! A backorder line is allocated from on-hand stock exactly like a strict line;
//! the quantity no location can cover is recorded as a reservation on the
//! default location, so `reserved_quantity` may exceed `stocked_quantity` while
//! the goods are owed. Strict lines keep the all-or-nothing behavior of
//! `InventoryService::reserve`.
//!
//! A backorder line whose variant has no inventory item is skipped: there is no
//! level to record the shortfall on, so the batch neither reserves it nor fails
//! for it. A strict line in that situation still conflicts.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, OrderId, StoreId,
    VariantId, now_timestamp,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, InventoryReservation, InventoryService, ReservationLine,
    ReservationLinePolicy, ReservationStatus,
};

const DEFAULT_LOCATION_ID: &str = "loc_default";

async fn setup() -> (StoreDatabase, InventoryService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = InventoryService::new(db.clone());
    (db, service)
}

/// Inserts a location with a fixed identifier so allocation order is testable.
async fn seed_location(db: &StoreDatabase, id: &str) {
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO inventory_locations (id, name, address, metadata, created_at, updated_at) \
         VALUES (?, ?, '{}', '{}', ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(now)
    .bind(now)
    .execute(db.pool())
    .await
    .unwrap();
}

async fn item_with_variant(service: &InventoryService, variant: &VariantId) -> InventoryItemId {
    service
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap()
        .id
}

async fn stock(service: &InventoryService, item: &InventoryItemId, location: &str, quantity: i64) {
    service
        .adjust_stock(
            item,
            &InventoryLocationId::from_string(location),
            quantity,
            "receiving",
            None,
        )
        .await
        .unwrap();
}

/// A line that may reserve beyond the on-hand stock.
fn backordered(variant: &VariantId, quantity: i64) -> ReservationLinePolicy {
    ReservationLinePolicy::may_backorder(ReservationLine::new(variant.clone(), quantity))
}

/// A line that must be covered by on-hand stock.
fn strict(variant: &VariantId, quantity: i64) -> ReservationLinePolicy {
    ReservationLinePolicy::strict(ReservationLine::new(variant.clone(), quantity))
}

/// The `(location, quantity)` allocations of a reservation batch.
fn allocations(reservations: &[InventoryReservation]) -> Vec<(String, i64)> {
    reservations
        .iter()
        .map(|reservation| {
            (
                reservation.location_id.as_str().to_owned(),
                reservation.quantity,
            )
        })
        .collect()
}

async fn default_level(service: &InventoryService, item: &InventoryItemId) -> (i64, i64) {
    let level = service
        .get_level(item, &InventoryLocationId::from_string(DEFAULT_LOCATION_ID))
        .await
        .unwrap();
    (level.stocked_quantity, level.reserved_quantity)
}

async fn level_count(db: &StoreDatabase, item: &InventoryItemId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM inventory_levels WHERE inventory_item_id = ?")
        .bind(item.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_backorder_line_books_the_shortfall_on_the_default_location() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 2).await;

    let line_item_id = LineItemId::new();
    let line = ReservationLine {
        variant_id: variant.clone(),
        quantity: 5,
        line_item_id: Some(line_item_id.clone()),
    };
    let checkout = CheckoutId::new();
    let reservations = service
        .reserve_lines(
            vec![ReservationLinePolicy::may_backorder(line)],
            fastcart_module_inventory::Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();

    assert_eq!(
        allocations(&reservations),
        vec![
            ("loc_aa".to_owned(), 2),
            (DEFAULT_LOCATION_ID.to_owned(), 3),
        ]
    );
    assert_eq!(
        reservations[1].line_item_id,
        Some(line_item_id),
        "the backorder keeps the line that caused it"
    );
    assert_eq!(reservations[1].status, ReservationStatus::Reserved);
    assert_eq!(default_level(&service, &item).await, (0, 3));
    // The backorder is owed, not available: availability never goes negative.
    assert_eq!(
        service
            .available_quantity(
                &item,
                &InventoryLocationId::from_string(DEFAULT_LOCATION_ID)
            )
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .available_quantity(&item, &InventoryLocationId::from_string("loc_aa"))
            .await
            .unwrap(),
        0
    );
    assert_eq!(service.variant_availability(&variant).await.unwrap(), 0);
}

#[tokio::test]
async fn a_backorder_line_within_stock_never_touches_the_default_location() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 5).await;

    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 2)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();

    assert_eq!(allocations(&reservations), vec![("loc_aa".to_owned(), 2)]);
    assert_eq!(level_count(&db, &item).await, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_locations WHERE id = ?")
            .bind(DEFAULT_LOCATION_ID)
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0,
        "a shortfall that never happens must not create the default location"
    );
}

#[tokio::test]
async fn backorder_lines_of_one_variant_share_the_default_location() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 1).await;

    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 3), backordered(&variant, 2)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();

    assert_eq!(
        allocations(&reservations),
        vec![
            ("loc_aa".to_owned(), 1),
            (DEFAULT_LOCATION_ID.to_owned(), 2),
            (DEFAULT_LOCATION_ID.to_owned(), 2),
        ]
    );
    assert_eq!(default_level(&service, &item).await, (0, 4));
}

#[tokio::test]
async fn committing_a_backorder_drives_the_level_negative() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 0).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 3)],
            fastcart_module_inventory::Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(default_level(&service, &item).await, (0, 3));

    assert_eq!(service.commit(&checkout, &order).await.unwrap(), 1);
    assert_eq!(
        default_level(&service, &item).await,
        (-3, 0),
        "the sold backorder units stay visible as negative stock"
    );
    let reservation = service.get_reservation(&reservations[0].id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::Committed);
    assert_eq!(reservation.order_id, Some(order.clone()));

    let movements = service.list_movements(&item).await.unwrap();
    let commit = movements.last().unwrap();
    assert_eq!(commit.quantity_delta, -3);
    assert_eq!(commit.reason, "commit");
    assert_eq!(commit.location_id.as_str(), DEFAULT_LOCATION_ID);
    assert_eq!(commit.reference.as_deref(), Some(order.as_str()));

    // Committing twice changes nothing.
    assert_eq!(service.commit(&checkout, &order).await.unwrap(), 0);
    assert_eq!(default_level(&service, &item).await, (-3, 0));
    assert_eq!(service.variant_availability(&variant).await.unwrap(), 0);
}

#[tokio::test]
async fn restocking_pulls_a_negative_level_back_toward_zero() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve_lines(
            vec![backordered(&variant, 5)],
            fastcart_module_inventory::Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    let default_location = InventoryLocationId::from_string(DEFAULT_LOCATION_ID);
    assert_eq!(default_level(&service, &item).await, (-5, 0));

    // A partial delivery is a valid state while units are still owed.
    let level = service
        .adjust_stock(&item, &default_location, 2, "restock", None)
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, -3);
    assert_eq!(level.reserved_quantity, 0);

    let level = service
        .adjust_stock(&item, &default_location, 3, "restock", None)
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, 0);
    assert_eq!(level.available_quantity(), 0);

    // Everything received beyond what was owed is sellable again.
    let level = service
        .adjust_stock(&item, &default_location, 4, "restock", None)
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, 4);
    assert_eq!(level.available_quantity(), 4);
    assert_eq!(service.variant_availability(&variant).await.unwrap(), 4);
}

#[tokio::test]
async fn restocking_covers_a_pending_backorder_first() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 0).await;

    service
        .reserve_lines(
            vec![backordered(&variant, 3)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();

    // The goods arrive while the checkout is still pending: they cover the
    // reservation instead of becoming available.
    let level = service
        .adjust_stock(
            &item,
            &InventoryLocationId::from_string(DEFAULT_LOCATION_ID),
            3,
            "restock",
            None,
        )
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, 3);
    assert_eq!(level.reserved_quantity, 3);
    assert_eq!(level.available_quantity(), 0);
}

#[tokio::test]
async fn a_strict_line_in_a_backorder_batch_still_fails_everything() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let strict_variant = VariantId::new();
    let backorder_variant = VariantId::new();
    let strict_item = item_with_variant(&service, &strict_variant).await;
    let backorder_item = item_with_variant(&service, &backorder_variant).await;
    stock(&service, &strict_item, "loc_aa", 1).await;
    stock(&service, &backorder_item, "loc_aa", 0).await;

    // The backordered line comes first: booking its shortfall before the strict
    // line is validated must still roll the whole batch back.
    let error = service
        .reserve_lines(
            vec![
                backordered(&backorder_variant, 1),
                strict(&strict_variant, 2),
            ],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains(strict_variant.as_str())),
        "unexpected error: {error}"
    );

    assert_eq!(
        service
            .available_quantity(&strict_item, &InventoryLocationId::from_string("loc_aa"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_reservations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(level_count(&db, &strict_item).await, 1);
    assert_eq!(level_count(&db, &backorder_item).await, 1);
}

#[tokio::test]
async fn a_backorder_line_without_an_item_is_skipped_instead_of_rejected() {
    let (db, service) = setup().await;
    let variant = VariantId::new();

    // There is no inventory item, so there is no ledger to record the line
    // against: the batch neither reserves it nor fails for it.
    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 1)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();
    assert!(reservations.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_reservations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_locations")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0,
        "the skip must not invent a location"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM inventory_items")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0,
        "the skip must not invent an item"
    );
}

#[tokio::test]
async fn a_strict_line_without_an_item_still_conflicts() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();

    let error = service
        .reserve_lines(
            vec![strict(&variant, 1)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains(variant.as_str())),
        "unexpected error: {error}"
    );

    // The strict-only entry point keeps the same contract.
    let error = service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 1)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains(variant.as_str())),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn releasing_a_backorder_returns_the_reserved_quantity() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;

    let checkout = CheckoutId::new();
    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 4)],
            fastcart_module_inventory::Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(default_level(&service, &item).await, (0, 4));

    assert_eq!(service.release_checkout(&checkout).await.unwrap(), 1);
    assert_eq!(default_level(&service, &item).await, (0, 0));
    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .status,
        ReservationStatus::Released
    );

    // The stock received afterwards is sellable again.
    let level = service
        .adjust_stock(
            &item,
            &InventoryLocationId::from_string(DEFAULT_LOCATION_ID),
            4,
            "restock",
            None,
        )
        .await
        .unwrap();
    assert_eq!(level.available_quantity(), 4);
}

#[tokio::test]
async fn strict_reserve_keeps_its_all_or_nothing_contract() {
    let (db, service) = setup().await;
    seed_location(&db, DEFAULT_LOCATION_ID).await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, DEFAULT_LOCATION_ID, 1).await;

    let error = service
        .reserve_lines(
            vec![strict(&variant, 2)],
            fastcart_module_inventory::Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains(variant.as_str())),
        "unexpected error: {error}"
    );
    assert_eq!(default_level(&service, &item).await, (1, 0));
}

#[tokio::test]
async fn canceling_a_backordered_order_returns_the_owed_units() {
    let (db, service) = setup().await;
    seed_location(&db, "loc_aa").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    stock(&service, &item, "loc_aa", 0).await;

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve_lines(
            vec![backordered(&variant, 3)],
            fastcart_module_inventory::Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(default_level(&service, &item).await, (-3, 0));

    // Cancelling the order gives the units the store no longer owes back to
    // the level, and releases the reservation.
    assert_eq!(service.restock_order(&order).await.unwrap(), 1);
    assert_eq!(default_level(&service, &item).await, (0, 0));
    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .status,
        ReservationStatus::Released
    );
    assert_eq!(service.restock_order(&order).await.unwrap(), 0);
    assert_eq!(default_level(&service, &item).await, (0, 0));
}
