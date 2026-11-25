use super::support::*;
use super::*;

#[tokio::test]
async fn receiving_a_return_restocks_exactly_once() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        8
    );
    let order_return = requested_return(&fixture.orders, &order, 2).await;

    let first = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(first.order_return.status, ReturnStatus::Received);
    assert_eq!(first.restocked_units, 2);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    let restocks = restock_movements(&movements);
    assert_eq!(restocks.len(), 1);
    assert_eq!(restocks[0].location_id, location_id);
    assert_eq!(restocks[0].quantity_delta, 2);

    // Repeating the receive is a no-op for inventory.
    let replay = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(replay.order_return.status, ReturnStatus::Received);
    assert_eq!(replay.restocked_units, 0);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 1);
}

#[tokio::test]
async fn partial_receipts_restock_each_new_quantity() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    let line = order.items[0].id.clone();

    let first = fixture
        .workflow
        .receive_return(
            &order.id,
            &order_return.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: line.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(first.order_return.status, ReturnStatus::PartiallyReceived);
    assert_eq!(first.restocked_units, 1);

    let second = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(second.order_return.status, ReturnStatus::Received);
    assert_eq!(second.restocked_units, 1);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 2);
}

#[tokio::test]
async fn concurrent_receives_never_double_restock() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;

    let (first, second) = tokio::join!(
        fixture
            .workflow
            .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default()),
        fixture
            .workflow
            .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default()),
    );
    let restocked = first.as_ref().map_or(0, |result| result.restocked_units)
        + second.as_ref().map_or(0, |result| result.restocked_units);
    assert_eq!(restocked, 2);

    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 1);
}

#[tokio::test]
async fn retry_after_a_failed_restock_restocks_once() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;

    // Simulate a crash after the order transaction committed the receive but
    // before the inventory restock ran.
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        8
    );

    let retried = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(retried.order_return.status, ReturnStatus::Received);
    assert_eq!(retried.restocked_units, 2);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );

    let replay = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(replay.restocked_units, 0);
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 1);
}

#[tokio::test]
async fn writing_off_received_units_reverses_the_restock() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );

    let written_off = fixture
        .orders
        .write_off_return(
            &order.id,
            &order_return.id,
            WriteOffReturnInput {
                items: vec![WriteOffReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(written_off.items[0].written_off_quantity, 1);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        9
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    let reversals: Vec<_> = movements
        .iter()
        .filter(|movement| movement.reason == "return_writeoff")
        .collect();
    assert_eq!(reversals.len(), 1);
    assert_eq!(reversals[0].quantity_delta, -1);
    assert_eq!(reversals[0].location_id, location_id);

    // Writing off the remaining unit reverses again, and replaying the
    // completed write-off does not reverse a third time.
    let completed = fixture
        .orders
        .write_off_return(&order.id, &order_return.id, WriteOffReturnInput::default())
        .await
        .unwrap();
    assert_eq!(completed.status, ReturnStatus::WrittenOff);
    let replay = fixture
        .orders
        .write_off_return(&order.id, &order_return.id, WriteOffReturnInput::default())
        .await
        .unwrap();
    assert_eq!(replay.status, ReturnStatus::WrittenOff);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        8
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(
        movements
            .iter()
            .filter(|movement| movement.reason == "return_writeoff")
            .count(),
        2
    );
}
