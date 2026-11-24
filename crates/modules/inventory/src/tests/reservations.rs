use super::*;

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
