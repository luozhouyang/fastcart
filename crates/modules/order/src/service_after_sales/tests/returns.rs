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

/// Places one single-line order with the given contact email.
async fn order_for(service: &OrderService, email: &str, quantity: i64) -> Order {
    let unit_price = 1_000;
    service
        .create_order(CreateOrderInput {
            email: email.to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

/// Creates a return on `order` and optionally submits it.
async fn return_for(
    service: &OrderService,
    order: &Order,
    quantity: i64,
    requested: bool,
) -> OrderReturn {
    let created = service
        .create_return(&order.id, return_input(order, quantity))
        .await
        .unwrap();
    if requested {
        service
            .request_return(&order.id, &created.id)
            .await
            .unwrap()
    } else {
        created
    }
}

#[tokio::test]
async fn store_return_list_pages_and_joins_order_context() {
    let service = service().await;
    let first = order_for(&service, "first@example.com", 2).await;
    let second = order_for(&service, "second@example.com", 1).await;
    let first_return = return_for(&service, &first, 1, false).await;
    let second_return = return_for(&service, &second, 1, true).await;

    let page = service
        .list_store_returns(Pagination::default(), StoreReturnFilter::default())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 2);
    assert_eq!(page.pagination.limit, Pagination::default().limit);
    assert_eq!(page.pagination.offset, 0);

    // Every row carries its order context and line count from the join.
    let first_row = page
        .data
        .iter()
        .find(|row| row.id == first_return.id)
        .expect("first return is listed");
    assert_eq!(first_row.order_id, first.id);
    assert_eq!(first_row.order_display_number, first.display_number);
    assert_eq!(first_row.customer_email, "first@example.com");
    assert_eq!(first_row.status, ReturnStatus::Draft);
    assert_eq!(first_row.reason.as_deref(), Some("damaged"));
    assert_eq!(first_row.item_count, 1);
    assert_eq!(first_row.created_at, first_return.created_at);
    assert_eq!(first_row.updated_at, first_return.updated_at);

    let second_row = page
        .data
        .iter()
        .find(|row| row.id == second_return.id)
        .expect("second return is listed");
    assert_eq!(second_row.status, ReturnStatus::Requested);
    assert_eq!(second_row.customer_email, "second@example.com");
    assert_eq!(second_row.order_display_number, second.display_number);

    // Paging one at a time returns each row exactly once.
    let mut seen = Vec::new();
    for offset in [0, 1] {
        let page = service
            .list_store_returns(
                Pagination { limit: 1, offset },
                StoreReturnFilter::default(),
            )
            .await
            .unwrap();
        assert_eq!(page.count, 2, "the count covers every page");
        assert_eq!(page.data.len(), 1);
        seen.push(page.data[0].id.clone());
    }
    seen.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    let mut expected = vec![first_return.id.clone(), second_return.id.clone()];
    expected.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    assert_eq!(seen, expected);

    // Past the end the page is empty but the total is unchanged.
    let empty = service
        .list_store_returns(
            Pagination {
                limit: 10,
                offset: 10,
            },
            StoreReturnFilter::default(),
        )
        .await
        .unwrap();
    assert!(empty.data.is_empty());
    assert_eq!(empty.count, 2);
}

#[tokio::test]
async fn store_return_list_filters_by_status() {
    let service = service().await;
    let first = order_for(&service, "first@example.com", 2).await;
    let second = order_for(&service, "second@example.com", 2).await;
    let third = order_for(&service, "third@example.com", 2).await;
    let draft = return_for(&service, &first, 1, false).await;
    let requested = return_for(&service, &second, 1, true).await;
    return_for(&service, &third, 1, true).await;

    let page = service
        .list_store_returns(
            Pagination::default(),
            StoreReturnFilter {
                status: Some(ReturnStatus::Requested),
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 2);
    assert!(
        page.data
            .iter()
            .all(|row| row.status == ReturnStatus::Requested)
    );
    assert!(page.data.iter().any(|row| row.id == requested.id));

    let drafts = service
        .list_store_returns(
            Pagination::default(),
            StoreReturnFilter {
                status: Some(ReturnStatus::Draft),
            },
        )
        .await
        .unwrap();
    assert_eq!(drafts.count, 1);
    assert_eq!(drafts.data[0].id, draft.id);

    let none = service
        .list_store_returns(
            Pagination::default(),
            StoreReturnFilter {
                status: Some(ReturnStatus::Canceled),
            },
        )
        .await
        .unwrap();
    assert_eq!(none.count, 0);
    assert!(none.data.is_empty());
}

#[tokio::test]
async fn store_return_list_validates_pagination() {
    let service = service().await;

    let error = service
        .list_store_returns(
            Pagination {
                limit: 0,
                offset: 0,
            },
            StoreReturnFilter::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let error = service
        .list_store_returns(
            Pagination {
                limit: Pagination::MAX_LIMIT + 1,
                offset: 0,
            },
            StoreReturnFilter::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let error = service
        .list_store_returns(
            Pagination {
                limit: 10,
                offset: -1,
            },
            StoreReturnFilter::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}
