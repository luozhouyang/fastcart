use super::*;

#[tokio::test]
async fn return_quantity_cannot_exceed_purchased() {
    let service = service().await;
    let order = order(&service, 2).await;

    let error = service
        .create_return(&order.id, return_input(&order, 3))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let created = service
        .create_return(&order.id, return_input(&order, 2))
        .await
        .unwrap();
    assert_eq!(created.status, ReturnStatus::Draft);

    let error = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn return_lifecycle_is_idempotent_and_quantity_checked() {
    let service = service().await;
    let order = order(&service, 2).await;
    let created = service
        .create_return(&order.id, return_input(&order, 2))
        .await
        .unwrap();

    let requested = service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    assert_eq!(requested.status, ReturnStatus::Requested);
    let again = service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    assert_eq!(again.status, ReturnStatus::Requested);

    // Receiving more than requested is rejected.
    let error = service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 3,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let partial = service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(partial.status, ReturnStatus::PartiallyReceived);
    assert_eq!(partial.items[0].received_quantity, 1);

    let received = service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(received.status, ReturnStatus::Received);
    assert_eq!(received.items[0].received_quantity, 2);

    // Repeating the receive after completion is a no-op.
    let again = service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(again.status, ReturnStatus::Received);

    let written_off = service
        .write_off_return(&order.id, &created.id, WriteOffReturnInput::default())
        .await
        .unwrap();
    assert_eq!(written_off.status, ReturnStatus::WrittenOff);
    assert_eq!(written_off.items[0].written_off_quantity, 2);

    let error = service
        .cancel_return(&order.id, &created.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn write_off_cannot_exceed_received() {
    let service = service().await;
    let order = order(&service, 3).await;
    let created = service
        .create_return(&order.id, return_input(&order, 3))
        .await
        .unwrap();
    service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();

    let error = service
        .write_off_return(
            &order.id,
            &created.id,
            WriteOffReturnInput {
                items: vec![crate::after_sales::WriteOffReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

/// A write-off reversal must not drive `stocked_quantity` negative. The
/// compensating UPDATE carries the same `stocked_quantity >= quantity`
/// guard as inventory commits, so a level that no longer holds the restock
/// yields a conflict and leaves stock untouched.
#[tokio::test]
async fn write_off_reversal_rejects_insufficient_stock() {
    let service = service().await;
    let variant_id = VariantId::new();
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                variant_id: Some(variant_id.clone()),
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let created = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    // Receiving normally restocks through InventoryService. Simulate that
    // restock movement, but leave the destination level already drained.
    let pool = service.database().pool();
    sqlx::query(
        "INSERT INTO inventory_locations \
             (id, name, address, metadata, created_at, updated_at) \
             VALUES ('loc_test', 'Test location', '{}', '{}', 0, 0)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_items \
             (id, sku, variant_id, metadata, created_at, updated_at) \
             VALUES ('item_test', NULL, ?, '{}', 0, 0)",
    )
    .bind(variant_id.as_str())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_levels \
             (id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
              incoming_quantity, created_at, updated_at) \
             VALUES ('level_test', 'item_test', 'loc_test', 0, 0, 0, 0, 0)",
    )
    .execute(pool)
    .await
    .unwrap();
    let restock_reference = format!(
        "return-restock:{}:{}:1",
        created.id.as_str(),
        order.items[0].id.as_str()
    );
    sqlx::query(
        "INSERT INTO inventory_movements \
             (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
              actor_id, metadata, created_at) \
             VALUES ('mv_restock', 'item_test', 'loc_test', 1, 'return_restock', ?, NULL, '{}', 1)",
    )
    .bind(&restock_reference)
    .execute(pool)
    .await
    .unwrap();

    let write_off = WriteOffReturnInput {
        items: vec![crate::after_sales::WriteOffReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity: 1,
        }],
    };
    let error = service
        .write_off_return(&order.id, &created.id, write_off.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    // The failed reversal rolled back: stock, movement and return item are
    // unchanged.
    let stock: i64 =
        sqlx::query_scalar("SELECT stocked_quantity FROM inventory_levels WHERE id = 'level_test'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stock, 0);
    let reversals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_movements WHERE reason = 'return_writeoff'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(reversals, 0);
    let written_off: i64 = sqlx::query_scalar(
        "SELECT written_off_quantity FROM order_return_items WHERE return_id = ?",
    )
    .bind(created.id.as_str())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(written_off, 0);

    // With the restocked unit present the same call succeeds.
    sqlx::query("UPDATE inventory_levels SET stocked_quantity = 1 WHERE id = 'level_test'")
        .execute(pool)
        .await
        .unwrap();
    let written_off = service
        .write_off_return(&order.id, &created.id, write_off)
        .await
        .unwrap();
    assert_eq!(written_off.status, ReturnStatus::WrittenOff);
    assert_eq!(written_off.items[0].written_off_quantity, 1);
    let stock: i64 =
        sqlx::query_scalar("SELECT stocked_quantity FROM inventory_levels WHERE id = 'level_test'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stock, 0);
}
