use super::support::*;
use super::*;

#[tokio::test]
async fn reducing_an_unshipped_line_returns_its_units() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;
    let stock_after_sale = STOCK - 4;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 3))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    assert_eq!(level(&fixture).await, stock_after_sale + 1);
    let movements = fixture
        .inventory
        .list_movements(&fixture.item_id)
        .await
        .unwrap();
    assert!(
        movements
            .iter()
            .any(|movement| { movement.reason == "order_edit" && movement.quantity_delta == 1 })
    );
}

#[tokio::test]
async fn increasing_a_line_or_adding_one_consumes_stock() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    let grow = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 5))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &grow.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, STOCK - 5);

    let add = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: Some(fixture.variant_id.clone()),
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 2,
                    requires_shipping: Some(true),
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, STOCK - 7);
}

#[tokio::test]
async fn a_manual_order_reduction_does_not_invent_stock() {
    let fixture = fixture().await;
    // No checkout ran, so the order never took the four units out of stock.
    let order = order(&fixture, 4, 1_000).await;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    assert_eq!(level(&fixture).await, STOCK);
}

#[tokio::test]
async fn an_edit_cannot_take_back_shipped_units() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;
    fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                }],
                partial: true,
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();

    // Preparing the edit already fails: two units are gone.
    let error = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    // A draft prepared before the shipment is re-checked at confirmation.
    let draft = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
                partial: true,
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &draft.id)
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(
        reloaded.items[0].quantity, 4,
        "the rejected edit changed nothing"
    );
}

#[tokio::test]
async fn insufficient_stock_rolls_the_edit_back() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;

    // Ask for ten more units when the shelf holds none.
    fixture
        .inventory
        .adjust_stock(
            &fixture.item_id,
            &fixture.location_id,
            -STOCK,
            "shrink",
            None,
        )
        .await
        .unwrap();

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: Some(fixture.variant_id.clone()),
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(1_000),
                    before_quantity: None,
                    after_quantity: 10,
                    requires_shipping: Some(true),
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items.len(), 1, "the added line must not survive");
    assert_eq!(reloaded.total, 1_000);
    let stored = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(
        stored.status,
        fastcart_module_order::OrderEditStatus::Draft,
        "a failed confirmation must leave the edit draft"
    );
}

#[tokio::test]
async fn replaying_a_confirmation_moves_stock_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let once = level(&fixture).await;
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, once, "replay must not restock twice");
}

#[tokio::test]
async fn growing_a_fulfilled_order_reopens_fulfillment() {
    let fixture = fixture().await;
    let order = order(&fixture, 2, 1_000).await;
    commit_stock(&fixture, &order, 2).await;
    let fulfilled = fixture
        .orders
        .fulfill(&order.id, FulfillmentInput::default())
        .await
        .unwrap();
    assert_eq!(fulfilled.status, OrderStatus::Fulfilled);
    assert_eq!(fulfilled.fulfillment_status, FulfillmentStatus::Fulfilled);

    // Add a unit: the order now holds one unit nobody shipped yet, so leaving
    // it `fulfilled` would make `fulfill` refuse to ship that unit.
    let add = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(true),
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(confirmed.order.status, OrderStatus::Processing);
    assert_eq!(
        confirmed.order.fulfillment_status,
        FulfillmentStatus::PartiallyFulfilled
    );

    let added_line = confirmed
        .order
        .items
        .iter()
        .find(|item| item.product_title == "Extra")
        .expect("the edit materialized the added line");
    let shipped = fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: added_line.id.clone(),
                    quantity: 1,
                }],
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(shipped.status, OrderStatus::Fulfilled);
    assert_eq!(shipped.fulfillment_status, FulfillmentStatus::Fulfilled);
}

#[tokio::test]
async fn stock_demand_is_summed_across_edit_lines() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    // Leave five units: each line fits on its own, but together they do not.
    fixture
        .inventory
        .adjust_stock(&fixture.item_id, &fixture.location_id, -15, "shrink", None)
        .await
        .unwrap();

    let add_line = |quantity: i64| OrderEditItemInput {
        order_item_id: None,
        action: OrderEditAction::Add,
        product_id: None,
        variant_id: Some(fixture.variant_id.clone()),
        product_title: Some("Extra".to_owned()),
        variant_title: None,
        sku: None,
        unit_price: Some(100),
        before_quantity: None,
        after_quantity: quantity,
        requires_shipping: Some(true),
        is_tax_inclusive: None,
    };
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![add_line(3), add_line(3)],
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("insufficient stock"), "{message}");

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items.len(), 1, "no part of the batch may apply");
}

#[tokio::test]
async fn a_mixed_edit_nets_stock_within_one_transaction() {
    let fixture = fixture().await;
    let order = order(&fixture, 2, 1_000).await;
    commit_stock(&fixture, &order, 2).await;
    let baseline = level(&fixture).await;

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![
                    edit(
                        OrderEditAction::Update,
                        Some(order.items[0].id.clone()),
                        Some(2),
                        1,
                    ),
                    OrderEditItemInput {
                        order_item_id: None,
                        action: OrderEditAction::Add,
                        product_id: None,
                        variant_id: Some(fixture.variant_id.clone()),
                        product_title: Some("Extra".to_owned()),
                        variant_title: None,
                        sku: None,
                        unit_price: Some(500),
                        before_quantity: None,
                        after_quantity: 1,
                        requires_shipping: Some(true),
                        is_tax_inclusive: None,
                    },
                ],
            },
        )
        .await
        .unwrap();
    let confirmed = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    assert_eq!(confirmed.order_edit.difference_total, Some(-500));
    assert_eq!(confirmed.order.total, 1_500);
    assert_eq!(
        level(&fixture).await,
        baseline,
        "the restocked unit and the consumed unit must cancel out"
    );
}
