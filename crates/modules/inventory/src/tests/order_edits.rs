use super::*;

#[tokio::test]
async fn duplicate_stock_references_are_rejected() {
    let (db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 5, "restock", None)
        .await
        .unwrap();

    let mut tx = db.begin().await.unwrap();
    let lines = vec![
        OrderEditStockLine::new(variant.clone(), 1, "edit:line"),
        OrderEditStockLine::new(variant, 1, "edit:line"),
    ];
    let error = InventoryService::apply_order_edit_stock(&mut tx, &OrderId::new(), &lines)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn a_zero_delta_stock_line_is_rejected() {
    let (db, service) = setup().await;
    let variant = VariantId::new();
    item_with_variant(&service, &variant).await;

    let mut tx = db.begin().await.unwrap();
    let lines = vec![OrderEditStockLine::new(variant, 0, "edit:zero")];
    let error = InventoryService::apply_order_edit_stock(&mut tx, &OrderId::new(), &lines)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn an_unmanaged_variant_moves_no_stock() {
    let (db, _) = setup().await;
    let order_id = OrderId::new();
    let unknown = VariantId::new();

    let mut tx = db.begin().await.unwrap();
    let lines = vec![OrderEditStockLine::new(unknown, 3, "edit:unknown")];
    let moved = InventoryService::apply_order_edit_stock(&mut tx, &order_id, &lines)
        .await
        .unwrap();
    assert_eq!(moved, 0);
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn a_replayed_stock_reference_moves_once() {
    let (db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 10, "restock", None)
        .await
        .unwrap();

    let order_id = OrderId::new();
    let checkout = CheckoutId::new();
    service
        .reserve(
            vec![ReservationLine::new(variant.clone(), 1)],
            Reference::new(checkout.clone()),
            3_600,
        )
        .await
        .unwrap();
    service.commit(&checkout, &order_id).await.unwrap();

    let line = OrderEditStockLine::new(
        variant.clone(),
        2,
        order_edit_stock_reference(order_id.as_str(), "edit_1", "line_1"),
    );

    let mut tx = db.begin().await.unwrap();
    let moved =
        InventoryService::apply_order_edit_stock(&mut tx, &order_id, std::slice::from_ref(&line))
            .await
            .unwrap();
    assert_eq!(moved, 2);
    tx.commit().await.unwrap();

    let mut tx = db.begin().await.unwrap();
    let replay = InventoryService::apply_order_edit_stock(&mut tx, &order_id, &[line])
        .await
        .unwrap();
    assert_eq!(replay, 0);
    tx.commit().await.unwrap();

    let level = service.get_level(&item, &location).await.unwrap();
    assert_eq!(level.stocked_quantity, 7);
}
