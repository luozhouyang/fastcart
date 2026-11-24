use super::*;

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
