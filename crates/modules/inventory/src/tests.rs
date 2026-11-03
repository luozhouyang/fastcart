//! Inventory service tests.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    Pagination, StoreId, VariantId, now_timestamp,
};

use crate::domain::{
    CreateInventoryItemInput, CreateLocationInput, Reference, ReservationLine, ReservationStatus,
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
