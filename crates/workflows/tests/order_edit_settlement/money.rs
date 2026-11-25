use super::support::*;
use super::*;

#[tokio::test]
async fn a_dearer_edit_becomes_payable_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

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
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                }],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let confirmed = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.difference_total, Some(500));
    assert_eq!(
        level(&fixture).await,
        STOCK,
        "a line that does not ship must not move stock"
    );

    let first = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap();
    assert_eq!(first.session.amount, 500);
    let second = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap();
    assert_eq!(
        first.session.id, second.session.id,
        "the difference must not open a second payable"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 1_500);
    assert_eq!(summary.captured_total, 1_000);
    assert_eq!(summary.outstanding, 500);
    assert_eq!(summary.refund_due(), 0);
}

#[tokio::test]
async fn a_cheaper_edit_refunds_what_was_captured_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

    // Remove the only line: the order now totals nothing and the whole
    // captured amount is owed back.
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("customer changed their mind".to_owned()),
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let confirmed = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.difference_total, Some(-1_000));

    let first = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert!(first.order_refund.is_some());
    assert_eq!(first.order_refund.as_ref().unwrap().amount, 1_000);

    let second = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(
        first.order_refund.as_ref().unwrap().id,
        second.order_refund.as_ref().unwrap().id,
        "the refund must be recorded once"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 0);
    assert_eq!(summary.captured_total, 1_000);
    assert_eq!(summary.refunded_total, 1_000);
    assert_eq!(summary.outstanding, 0);
}

#[tokio::test]
async fn a_cheaper_edit_refunds_past_a_lowered_but_nonzero_total() {
    let fixture = fixture().await;
    // Two digital lines, so the test is about money only.
    let order = fixture
        .orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![
                CreateOrderItemInput {
                    product_title: "Expensive".to_owned(),
                    unit_price: 1_000,
                    quantity: 1,
                    requires_shipping: false,
                    ..CreateOrderItemInput::default()
                },
                CreateOrderItemInput {
                    product_title: "Cheap".to_owned(),
                    unit_price: 500,
                    quantity: 1,
                    requires_shipping: false,
                    ..CreateOrderItemInput::default()
                },
            ],
            subtotal: 1_500,
            total: 1_500,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let order = fixture.orders.confirm(&order.id).await.unwrap();
    pay(&fixture, &order).await;

    // Removing the expensive line leaves a total (500) below the refund the
    // edit owes (1000). The refund must be measured against what was captured,
    // not against the lowered total.
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("dropped the expensive line".to_owned()),
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let refund = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(refund.order_refund.as_ref().unwrap().amount, 1_000);

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 500);
    assert_eq!(summary.captured_total, 1_500);
    assert_eq!(summary.refunded_total, 1_000);
    assert_eq!(summary.outstanding, 0);
}

#[tokio::test]
async fn a_stale_draft_settles_only_its_own_difference() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    // Two drafts are prepared against the same 4000 total. Confirming the
    // addition first changes the order; the reduction's difference must then
    // measure against 4500 and settle only the 1000 it removed, not include
    // the addition a second time.
    let reduce = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 3))
        .await
        .unwrap();
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
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                }],
            },
        )
        .await
        .unwrap();

    let added = fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(added.order_edit.difference_total, Some(500));

    let reduced = fixture
        .workflow
        .confirm_order_edit(&order.id, &reduce.id)
        .await
        .unwrap();
    assert_eq!(reduced.order_edit.difference_total, Some(-1_000));
    assert_eq!(reduced.order.total, 3_500);
}

#[tokio::test]
async fn collecting_a_cheaper_edit_is_refused() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let error = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.outstanding, -1_000);
    assert_eq!(summary.refund_due(), 1_000);
}
