use super::support::*;
use super::*;

#[tokio::test]
async fn accepting_an_exchange_restocks_inbound_items_once() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(original.clone()), true).await;
    let (inventory, original_item, location) =
        stock_order(&fixture.db, &order, &original, 10, 2).await;
    assert_eq!(
        inventory
            .available_quantity(&original_item, &location)
            .await
            .unwrap(),
        8
    );

    let exchange = fixture
        .orders
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(replacement.clone()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let first = fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();
    assert_eq!(first.restocked_units, 1);
    assert_eq!(
        inventory
            .available_quantity(&original_item, &location)
            .await
            .unwrap(),
        9
    );

    let replay = fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();
    assert_eq!(replay.restocked_units, 0);
    assert_eq!(
        inventory
            .available_quantity(&original_item, &location)
            .await
            .unwrap(),
        9
    );
    let movements = inventory.list_movements(&original_item).await.unwrap();
    assert_eq!(
        movements
            .iter()
            .filter(|movement| movement.reason == "exchange_restock")
            .count(),
        1
    );
}

#[tokio::test]
async fn exchange_outbound_fulfillment_is_idempotent() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(original.clone()), true).await;
    let (inventory, _, location) = stock_order(&fixture.db, &order, &original, 10, 2).await;
    let replacement_item =
        stock_replacement_variant(&inventory, &order.id, &replacement, &location, 5).await;

    let exchange = fixture
        .orders
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(replacement.clone()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let first = fixture
        .workflow
        .fulfill_exchange(&order.id, &exchange.id, "manual", vec!["OUT-1".to_owned()])
        .await
        .unwrap();
    assert_eq!(first.consumed_units, 1);
    let fulfillment = first.fulfillment.as_ref().unwrap();
    assert_eq!(fulfillment.items[0].variant_id, replacement);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        3
    );

    let replay = fixture
        .workflow
        .fulfill_exchange(&order.id, &exchange.id, "manual", Vec::new())
        .await
        .unwrap();
    assert_eq!(replay.consumed_units, 0);
    assert_eq!(replay.fulfillment.as_ref().unwrap().id, fulfillment.id);
    assert_eq!(
        inventory
            .list_movements(&replacement_item)
            .await
            .unwrap()
            .iter()
            .filter(|movement| movement.reason == "after_sales_consume")
            .count(),
        1
    );
}
