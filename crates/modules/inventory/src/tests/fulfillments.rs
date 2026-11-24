use super::*;

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
