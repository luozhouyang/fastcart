use super::*;

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
