//! Inventory service tests.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    Pagination, ReturnId, StoreId, VariantId, now_timestamp,
};

use crate::domain::{
    CreateInventoryItemInput, CreateLocationInput, ExchangeRestockLine, Reference, ReservationLine,
    ReservationStatus, ReturnRestockLine, StockConsumption, exchange_restock_reference,
    return_restock_reference,
};
use crate::service::InventoryService;

async fn setup() -> (StoreDatabase, InventoryService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = InventoryService::new(db.clone());
    (db, service)
}

async fn location(service: &InventoryService, name: &str) -> InventoryLocationId {
    let created = service
        .create_location(CreateLocationInput {
            name: name.to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    created.id
}

async fn item_with_variant(service: &InventoryService, variant: &VariantId) -> InventoryItemId {
    let created = service
        .create_item(CreateInventoryItemInput {
            sku: Some(format!("sku-{}", variant.as_str())),
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    created.id
}

#[tokio::test]
async fn locations_crud_and_pagination() {
    let (_db, service) = setup().await;
    let first = service
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let second = service
        .create_location(CreateLocationInput {
            name: "Overflow".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();

    let page = service
        .list_locations(Pagination {
            limit: 1,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let updated = service
        .update_location(
            &first.id,
            crate::domain::UpdateLocationInput {
                name: Some("Main warehouse".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Main warehouse");
    assert_eq!(
        service.get_location(&second.id).await.unwrap().name,
        "Overflow"
    );

    assert!(matches!(
        service.get_location(&InventoryLocationId::new()).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn item_creation_enforces_unique_variant_and_sku() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let item = service
        .create_item(CreateInventoryItemInput {
            sku: Some("abc".to_owned()),
            variant_id: Some(variant.clone()),
            metadata: Some(Metadata::empty()),
        })
        .await
        .unwrap();

    assert_eq!(
        service.get_item_by_variant(&variant).await.unwrap().id,
        item.id
    );
    assert_eq!(
        service.get_item(&item.id).await.unwrap().sku.as_deref(),
        Some("abc")
    );

    assert!(matches!(
        service
            .create_item(CreateInventoryItemInput {
                sku: Some("def".to_owned()),
                variant_id: Some(variant.clone()),
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        service
            .create_item(CreateInventoryItemInput {
                sku: Some("abc".to_owned()),
                variant_id: Some(VariantId::new()),
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));

    let other = service
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: None,
            metadata: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        service.attach_variant(&other.id, &variant).await,
        Err(AppError::Conflict(_))
    ));
    let attached = service
        .attach_variant(&other.id, &VariantId::new())
        .await
        .unwrap();
    assert!(attached.variant_id.is_some());
}

#[tokio::test]
async fn adjust_stock_updates_levels_and_records_movements() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let item = item_with_variant(&service, &VariantId::new()).await;

    let level = service
        .adjust_stock(&item, &location, 10, "receiving", Some("user_1"))
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, 10);
    assert_eq!(level.reserved_quantity, 0);
    assert_eq!(level.available_quantity(), 10);

    let level = service
        .adjust_stock(&item, &location, -4, "shrinkage", None)
        .await
        .unwrap();
    assert_eq!(level.stocked_quantity, 6);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        6
    );

    let movements = service.list_movements(&item).await.unwrap();
    assert_eq!(movements.len(), 2);
    assert_eq!(movements[0].quantity_delta, 10);
    assert_eq!(movements[0].reason, "receiving");
    assert_eq!(movements[0].actor_id.as_deref(), Some("user_1"));
    assert_eq!(movements[1].quantity_delta, -4);

    assert!(matches!(
        service
            .adjust_stock(&item, &location, -7, "breakage", None)
            .await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        6
    );
}

#[tokio::test]
async fn adjustment_cannot_breach_reserved_quantity() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 5, "receiving", None)
        .await
        .unwrap();

    let checkout = CheckoutId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant, 3)],
            Reference::new(checkout),
            60,
        )
        .await
        .unwrap();

    assert!(matches!(
        service
            .adjust_stock(&item, &location, -3, "shrinkage", None)
            .await,
        Err(AppError::Conflict(_))
    ));
    let level = service.get_level(&item, &location).await.unwrap();
    assert_eq!(level.available_quantity(), 2);
}

#[tokio::test]
async fn transfer_moves_stock_and_validates() {
    let (_db, service) = setup().await;
    let source = location(&service, "Source").await;
    let destination = location(&service, "Destination").await;
    let item = item_with_variant(&service, &VariantId::new()).await;
    service
        .adjust_stock(&item, &source, 8, "receiving", None)
        .await
        .unwrap();

    let (from, to) = service
        .transfer(&item, &source, &destination, 5)
        .await
        .unwrap();
    assert_eq!(from.stocked_quantity, 3);
    assert_eq!(to.stocked_quantity, 5);

    let movements = service.list_movements(&item).await.unwrap();
    assert_eq!(movements.len(), 3);
    assert_eq!(movements[1].quantity_delta, -5);
    assert_eq!(movements[2].quantity_delta, 5);

    assert!(matches!(
        service.transfer(&item, &source, &source, 1).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service.transfer(&item, &source, &destination, 0).await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service.transfer(&item, &source, &destination, 4).await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn reservation_prefers_the_default_location() {
    let (db, service) = setup().await;
    let now = now_timestamp();
    for id in ["loc_default", "loc_secondary"] {
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
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    let default = InventoryLocationId::from_string("loc_default");
    let secondary = InventoryLocationId::from_string("loc_secondary");
    service
        .adjust_stock(&item, &default, 2, "receiving", None)
        .await
        .unwrap();
    service
        .adjust_stock(&item, &secondary, 5, "receiving", None)
        .await
        .unwrap();

    let reservations = service
        .reserve(
            vec![ReservationLine::new(variant, 3)],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(reservations.len(), 2);
    assert_eq!(reservations[0].location_id, default);
    assert_eq!(reservations[0].quantity, 2);
    assert_eq!(reservations[1].location_id, secondary);
    assert_eq!(reservations[1].quantity, 1);
}

#[tokio::test]
async fn reservation_allocates_across_locations_deterministically() {
    let (_db, service) = setup().await;
    let first = location(&service, "A").await;
    let second = location(&service, "B").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &first, 2, "receiving", None)
        .await
        .unwrap();
    service
        .adjust_stock(&item, &second, 2, "receiving", None)
        .await
        .unwrap();

    let reservations = service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 4)],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(reservations.iter().map(|r| r.quantity).sum::<i64>(), 4);
    assert_eq!(service.variant_availability(&variant).await.unwrap(), 0);
}

#[tokio::test]
async fn release_restores_availability() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 4, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    let reservations = service
        .reserve(
            vec![ReservationLine {
                variant_id: variant.clone(),
                quantity: 3,
                line_item_id: Some(LineItemId::new()),
            }],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        1
    );

    let released = service.release_checkout(&checkout).await.unwrap();
    assert_eq!(released, 1);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        4
    );
    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .status,
        ReservationStatus::Released
    );

    assert_eq!(
        service
            .release(&[reservations[0].id.clone()])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn commit_removes_stock_and_records_movement() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 10, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve(
            vec![ReservationLine::new(variant, 4)],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();

    assert_eq!(service.commit(&checkout, &order).await.unwrap(), 1);
    let level = service.get_level(&item, &location).await.unwrap();
    assert_eq!(level.stocked_quantity, 6);
    assert_eq!(level.reserved_quantity, 0);
    let reservation = service.get_reservation(&reservations[0].id).await.unwrap();
    assert_eq!(reservation.status, ReservationStatus::Committed);
    assert_eq!(reservation.order_id, Some(order.clone()));

    let movements = service.list_movements(&item).await.unwrap();
    assert_eq!(movements.len(), 2);
    assert_eq!(movements[1].quantity_delta, -4);
    assert_eq!(movements[1].reason, "commit");
    assert_eq!(movements[1].reference.as_deref(), Some(order.as_str()));

    assert_eq!(service.commit(&checkout, &order).await.unwrap(), 0);
}

#[tokio::test]
async fn restocking_a_canceled_order_returns_committed_stock() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 10, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    let order = OrderId::new();
    let reservations = service
        .reserve(
            vec![ReservationLine::new(variant, 3)],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        7
    );

    assert_eq!(service.restock_order(&order).await.unwrap(), 1);
    let level = service.get_level(&item, &location).await.unwrap();
    assert_eq!(level.stocked_quantity, 10);
    assert_eq!(level.reserved_quantity, 0);
    assert_eq!(
        service
            .get_reservation(&reservations[0].id)
            .await
            .unwrap()
            .status,
        ReservationStatus::Released
    );
    let movements = service.list_movements(&item).await.unwrap();
    let restock = movements.last().unwrap();
    assert_eq!(restock.quantity_delta, 3);
    assert_eq!(restock.reason, "order_canceled");
    assert_eq!(restock.reference.as_deref(), Some(order.as_str()));

    // Idempotent: an already restocked order cannot add stock twice.
    assert_eq!(service.restock_order(&order).await.unwrap(), 0);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        10
    );
}

#[tokio::test]
async fn expired_reservations_are_released() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 2, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant, 2)],
            Reference::new(checkout.clone()),
            30,
        )
        .await
        .unwrap();
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        0
    );

    let released = service.expire_due(now_timestamp() + 31).await.unwrap();
    assert_eq!(released, 1);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        2
    );
    assert_eq!(service.expire_due(now_timestamp() + 31).await.unwrap(), 0);
}

#[tokio::test]
async fn insufficient_stock_is_a_conflict() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 1, "receiving", None)
        .await
        .unwrap();

    let error = service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 2)],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(message) if message.contains(variant.as_str())));
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn multi_line_partial_failure_rolls_back_everything() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let rich_variant = VariantId::new();
    let empty_variant = VariantId::new();
    let rich_item = item_with_variant(&service, &rich_variant).await;
    let empty_item = item_with_variant(&service, &empty_variant).await;
    service
        .adjust_stock(&rich_item, &location, 5, "receiving", None)
        .await
        .unwrap();
    service
        .adjust_stock(&empty_item, &location, 0, "receiving", None)
        .await
        .unwrap();

    let error = service
        .reserve(
            vec![
                ReservationLine::new(rich_variant, 3),
                ReservationLine::new(empty_variant.clone(), 1),
            ],
            Reference::new(CheckoutId::new()),
            60,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert_eq!(
        service
            .available_quantity(&rich_item, &location)
            .await
            .unwrap(),
        5
    );
    assert_eq!(
        service
            .available_quantity(&empty_item, &location)
            .await
            .unwrap(),
        0
    );
    assert_eq!(service.list_movements(&rich_item).await.unwrap().len(), 1);
}

#[tokio::test]
async fn concurrent_reservations_never_oversell_the_last_unit() {
    let (_db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 1, "receiving", None)
        .await
        .unwrap();

    let service = Arc::new(service);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let service = Arc::clone(&service);
        let variant = variant.clone();
        handles.push(tokio::spawn(async move {
            service
                .reserve(
                    vec![ReservationLine::new(variant, 1)],
                    Reference::new(CheckoutId::new()),
                    60,
                )
                .await
                .is_ok()
        }));
    }

    let mut successes = 0;
    for handle in handles {
        if handle.await.unwrap() {
            successes += 1;
        }
    }
    assert_eq!(successes, 1);
    assert_eq!(
        service.available_quantity(&item, &location).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn reservation_validation_rejects_bad_input() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let _item = item_with_variant(&service, &variant).await;

    assert!(matches!(
        service
            .reserve(Vec::new(), Reference::new(CheckoutId::new()), 60)
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .reserve(
                vec![ReservationLine::new(variant.clone(), 0)],
                Reference::new(CheckoutId::new()),
                60,
            )
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .reserve(
                vec![ReservationLine::new(variant, 1)],
                Reference::new(CheckoutId::new()),
                0,
            )
            .await,
        Err(AppError::Validation(_))
    ));
    assert!(matches!(
        service
            .adjust_stock(
                &InventoryItemId::new(),
                &InventoryLocationId::new(),
                1,
                "receiving",
                None,
            )
            .await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn availability_sums_levels_and_ignores_unknown_variants() {
    let (_db, service) = setup().await;
    let first = location(&service, "A").await;
    let second = location(&service, "B").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &first, 3, "receiving", None)
        .await
        .unwrap();
    service
        .adjust_stock(&item, &second, 6, "receiving", None)
        .await
        .unwrap();

    assert_eq!(service.variant_availability(&variant).await.unwrap(), 9);
    assert_eq!(service.list_levels(&item).await.unwrap().len(), 2);
    assert_eq!(
        service
            .list_levels_for_location(&first)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .variant_availability(&VariantId::new())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .available_quantity(&item, &InventoryLocationId::new())
            .await
            .unwrap_err()
            .code(),
        "NOT_FOUND"
    );
}

#[tokio::test]
async fn return_restock_is_idempotent_and_prefers_the_committed_location() {
    let (_db, service) = setup().await;
    // Stock only a secondary location so the committed reservation points
    // there instead of the default location.
    let secondary = location(&service, "Secondary").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &secondary, 10, "receiving", None)
        .await
        .unwrap();

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 3)],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        7
    );

    let return_id = ReturnId::new();
    let order_item_id = LineItemId::new();
    let line = ReturnRestockLine::new(order_item_id.clone(), variant, 2, 2);
    assert_eq!(
        service
            .restock_return(&order, &return_id, std::slice::from_ref(&line))
            .await
            .unwrap(),
        2
    );
    let level = service.get_level(&item, &secondary).await.unwrap();
    assert_eq!(level.stocked_quantity, 9);
    assert!(
        service
            .get_level(&item, &InventoryLocationId::from_string("loc_default"))
            .await
            .is_err()
    );

    // Replaying the same receive records no second movement.
    assert_eq!(
        service
            .restock_return(&order, &return_id, std::slice::from_ref(&line))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .get_level(&item, &secondary)
            .await
            .unwrap()
            .stocked_quantity,
        9
    );
    let movements = service.list_movements(&item).await.unwrap();
    let restocks: Vec<_> = movements
        .iter()
        .filter(|movement| movement.reason == "return_restock")
        .collect();
    assert_eq!(restocks.len(), 1);
    assert_eq!(restocks[0].quantity_delta, 2);
    assert_eq!(
        restocks[0].reference.as_deref(),
        Some(return_restock_reference(&return_id, &order_item_id, 2).as_str())
    );
    assert_eq!(restocks[0].location_id, secondary);
}

#[tokio::test]
async fn return_restock_falls_back_to_the_default_location() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    let order = OrderId::new();
    let return_id = ReturnId::new();

    let line = ReturnRestockLine::new(LineItemId::new(), variant.clone(), 3, 3);
    assert_eq!(
        service
            .restock_return(&order, &return_id, &[line])
            .await
            .unwrap(),
        3
    );
    let default = InventoryLocationId::from_string("loc_default");
    let level = service.get_level(&item, &default).await.unwrap();
    assert_eq!(level.stocked_quantity, 3);

    // A variant without an inventory item is ignored.
    let unknown = ReturnRestockLine::new(LineItemId::new(), VariantId::new(), 5, 5);
    assert_eq!(
        service
            .restock_return(&order, &return_id, &[unknown])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn distinct_partial_receipts_restock_once_each() {
    let (_db, service) = setup().await;
    let default = InventoryLocationId::from_string("loc_default");
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    let order = OrderId::new();
    let return_id = ReturnId::new();
    let order_item_id = LineItemId::new();

    let first = ReturnRestockLine::new(order_item_id.clone(), variant.clone(), 1, 1);
    let second = ReturnRestockLine::new(order_item_id, variant, 2, 3);
    assert_eq!(
        service
            .restock_return(&order, &return_id, &[first])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .restock_return(&order, &return_id, std::slice::from_ref(&second))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        service
            .restock_return(&order, &return_id, &[second])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .get_level(&item, &default)
            .await
            .unwrap()
            .stocked_quantity,
        3
    );
    assert_eq!(
        service
            .list_movements(&item)
            .await
            .unwrap()
            .iter()
            .filter(|movement| movement.reason == "return_restock")
            .count(),
        2
    );
}

#[tokio::test]
async fn concurrent_repeated_restocks_do_not_duplicate_movements() {
    let (_db, service) = setup().await;
    let default = InventoryLocationId::from_string("loc_default");
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    let order = OrderId::new();
    let return_id = ReturnId::new();
    let line = ReturnRestockLine::new(LineItemId::new(), variant, 2, 2);

    let service = Arc::new(service);
    let first_service = Arc::clone(&service);
    let second_service = Arc::clone(&service);
    let first_line = line.clone();
    let second_line = line;
    let (first, second) = tokio::join!(
        first_service.restock_return(&order, &return_id, std::slice::from_ref(&first_line)),
        second_service.restock_return(&order, &return_id, std::slice::from_ref(&second_line)),
    );

    assert_eq!(first.unwrap() + second.unwrap(), 2);
    assert_eq!(
        service
            .get_level(&item, &default)
            .await
            .unwrap()
            .stocked_quantity,
        2
    );
    assert_eq!(
        service
            .list_movements(&item)
            .await
            .unwrap()
            .iter()
            .filter(|movement| movement.reason == "return_restock")
            .count(),
        1
    );
}

#[tokio::test]
async fn consuming_replacement_stock_is_idempotent_and_rejects_shortages() {
    let (_db, service) = setup().await;
    let secondary = location(&service, "Secondary").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &secondary, 5, "receiving", None)
        .await
        .unwrap();

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 2)],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        3
    );

    let line = StockConsumption::new(variant.clone(), 2, "test-consume:line-1");
    assert_eq!(
        service
            .consume_stock(&order, std::slice::from_ref(&line))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        1
    );

    // Replaying the same reference consumes nothing more.
    assert_eq!(
        service
            .consume_stock(&order, std::slice::from_ref(&line))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        1
    );
    let movements = service.list_movements(&item).await.unwrap();
    let consumed: Vec<_> = movements
        .iter()
        .filter(|movement| movement.reason == "after_sales_consume")
        .collect();
    assert_eq!(consumed.len(), 1);
    assert_eq!(consumed[0].quantity_delta, -2);
    assert_eq!(consumed[0].location_id, secondary);

    // A shortage rolls the batch back without recording a movement.
    let shortage = StockConsumption::new(variant, 5, "test-consume:line-2");
    assert!(matches!(
        service.consume_stock(&order, &[shortage]).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        1
    );
    assert_eq!(
        service
            .list_movements(&item)
            .await
            .unwrap()
            .iter()
            .filter(|movement| movement.reason == "after_sales_consume")
            .count(),
        1
    );
}

#[tokio::test]
async fn exchange_restock_is_idempotent_and_prefers_the_committed_location() {
    let (_db, service) = setup().await;
    let secondary = location(&service, "Secondary").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &secondary, 2, "receiving", None)
        .await
        .unwrap();

    let checkout = CheckoutId::new();
    let order = OrderId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 1)],
            Reference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order).await.unwrap();
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        1
    );

    let exchange = "exch_test";
    let line = ExchangeRestockLine::new("evt_line", variant, 1);
    assert_eq!(
        service
            .restock_exchange(&order, exchange, std::slice::from_ref(&line))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        service
            .restock_exchange(&order, exchange, std::slice::from_ref(&line))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service.available_quantity(&item, &secondary).await.unwrap(),
        2
    );
    let movements = service.list_movements(&item).await.unwrap();
    let restocks: Vec<_> = movements
        .iter()
        .filter(|movement| movement.reason == "exchange_restock")
        .collect();
    assert_eq!(restocks.len(), 1);
    assert_eq!(restocks[0].quantity_delta, 1);
    assert_eq!(restocks[0].location_id, secondary);
    assert_eq!(
        restocks[0].reference.as_deref(),
        Some(exchange_restock_reference(exchange, "evt_line").as_str())
    );
}

#[tokio::test]
async fn concurrent_item_creation_maps_unique_violations_to_conflict() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let input = CreateInventoryItemInput {
        sku: Some("race-sku".to_owned()),
        variant_id: Some(variant),
        metadata: None,
    };
    let (first, second) = tokio::join!(
        service.create_item(input.clone()),
        service.create_item(input),
    );

    let successes = usize::from(first.is_ok()) + usize::from(second.is_ok());
    assert_eq!(
        successes, 1,
        "exactly one concurrent creation should win: {first:?} / {second:?}"
    );
    assert!(
        matches!(&first, Err(AppError::Conflict(_)))
            || matches!(&second, Err(AppError::Conflict(_))),
        "the loser must surface a conflict: {first:?} / {second:?}"
    );
    assert!(!matches!(&first, Err(AppError::Internal(_))));
    assert!(!matches!(&second, Err(AppError::Internal(_))));
}

#[tokio::test]
async fn concurrent_variant_attach_maps_unique_violations_to_conflict() {
    let (_db, service) = setup().await;
    let first = service
        .create_item(CreateInventoryItemInput::default())
        .await
        .unwrap();
    let second = service
        .create_item(CreateInventoryItemInput::default())
        .await
        .unwrap();
    let variant = VariantId::new();
    let (left, right) = tokio::join!(
        service.attach_variant(&first.id, &variant),
        service.attach_variant(&second.id, &variant),
    );

    let successes = usize::from(left.is_ok()) + usize::from(right.is_ok());
    assert_eq!(
        successes, 1,
        "exactly one concurrent attach should win: {left:?} / {right:?}"
    );
    assert!(
        matches!(&left, Err(AppError::Conflict(_))) || matches!(&right, Err(AppError::Conflict(_))),
        "the loser must surface a conflict: {left:?} / {right:?}"
    );
    assert!(!matches!(&left, Err(AppError::Internal(_))));
    assert!(!matches!(&right, Err(AppError::Internal(_))));
}
