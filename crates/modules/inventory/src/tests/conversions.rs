use super::*;

use crate::domain::OrderConversionStockLine;

#[tokio::test]
async fn a_conversion_commits_stock_and_records_the_order_line() {
    let (db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 10, "restock", None)
        .await
        .unwrap();
    let order_id = OrderId::new();
    let line_item_id = LineItemId::new();

    let mut tx = db.begin().await.unwrap();
    let committed = InventoryService::commit_order_conversion_stock(
        &mut tx,
        &order_id,
        &[OrderConversionStockLine::new(
            variant.clone(),
            line_item_id.clone(),
            3,
            false,
        )],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(committed, 3);
    assert_eq!(
        service
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        7
    );
    let (status, line_item, order): (String, String, String) = sqlx::query_as(
        "SELECT status, line_item_id, order_id FROM inventory_reservations WHERE order_id = ?",
    )
    .bind(order_id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(status, "committed");
    assert_eq!(line_item, line_item_id.as_str());
    assert_eq!(order, order_id.as_str());
    let (reason, reference): (String, String) = sqlx::query_as(
        "SELECT reason, reference FROM inventory_movements \
         WHERE inventory_item_id = ? AND reason = 'commit'",
    )
    .bind(item.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(reason, "commit");
    assert_eq!(reference, order_id.as_str());

    // Replaying the same line takes nothing further.
    let mut tx = db.begin().await.unwrap();
    let replayed = InventoryService::commit_order_conversion_stock(
        &mut tx,
        &order_id,
        &[OrderConversionStockLine::new(
            variant,
            line_item_id,
            3,
            false,
        )],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(replayed, 0);
    assert_eq!(
        service
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        7
    );
}

#[tokio::test]
async fn a_backordered_conversion_records_the_shortfall_on_the_default_level() {
    let (db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 1, "restock", None)
        .await
        .unwrap();
    let order_id = OrderId::new();

    let mut tx = db.begin().await.unwrap();
    let committed = InventoryService::commit_order_conversion_stock(
        &mut tx,
        &order_id,
        &[OrderConversionStockLine::new(
            variant.clone(),
            LineItemId::new(),
            3,
            true,
        )],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(committed, 3);
    assert_eq!(
        service
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        0
    );
    let default_location = InventoryLocationId::from_string("loc_default".to_owned());
    assert_eq!(
        service
            .get_level(&item, &default_location)
            .await
            .unwrap()
            .stocked_quantity,
        -2
    );
    assert_eq!(service.variant_availability(&variant).await.unwrap(), 0);
}

#[tokio::test]
async fn a_strict_conversion_short_on_stock_writes_nothing() {
    let (db, service) = setup().await;
    let location = location(&service, "Main").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &location, 1, "restock", None)
        .await
        .unwrap();
    let order_id = OrderId::new();

    let mut tx = db.begin().await.unwrap();
    let error = InventoryService::commit_order_conversion_stock(
        &mut tx,
        &order_id,
        &[OrderConversionStockLine::new(
            variant,
            LineItemId::new(),
            2,
            false,
        )],
    )
    .await
    .unwrap_err();
    tx.rollback().await.unwrap();

    assert!(matches!(error, AppError::Conflict(_)), "{error:?}");
    assert_eq!(
        service
            .get_level(&item, &location)
            .await
            .unwrap()
            .stocked_quantity,
        1
    );
    let reservations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_reservations WHERE order_id = ?")
            .bind(order_id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(reservations, 0);
}
