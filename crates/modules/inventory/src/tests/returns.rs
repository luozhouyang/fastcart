use super::*;

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
