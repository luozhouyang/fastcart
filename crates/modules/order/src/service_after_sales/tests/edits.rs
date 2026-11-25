use super::*;

#[tokio::test]
async fn order_edits_recalculate_totals_and_materialize_the_line_changes() {
    let service = service().await;
    let order = order(&service, 2).await;

    let added = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("add gift".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: Some("GIFT-1".to_owned()),
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(added.status, OrderEditStatus::Draft);
    assert_eq!(added.before_total, 2_000);
    assert_eq!(added.after_total, 2_500);
    assert_eq!(added.items[0].before_quantity, 0);
    assert_eq!(added.items[0].after_quantity, 1);

    let confirmed = service
        .confirm_order_edit(&order.id, &added.id)
        .await
        .unwrap();
    assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.total, 2_500);
    // The edit materializes: the added line exists as an order item and the
    // order totals are the sums of the materialized lines.
    assert_eq!(reloaded.items.len(), 2);
    let added_line = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be materialized");
    assert_eq!(added_line.quantity, 1);
    assert_eq!(added_line.unit_price, 500);
    assert_eq!(added_line.subtotal, 500);
    assert_eq!(added_line.total, 500);
    let purchased = reloaded
        .items
        .iter()
        .find(|item| item.sku.is_none())
        .expect("the purchased line must survive the edit");
    assert_eq!(purchased.quantity, 2);
    assert_eq!(purchased.unit_price, 1_000);
    assert_eq!(purchased.subtotal, 2_000);
    assert_eq!(
        reloaded.items.iter().map(|item| item.subtotal).sum::<i64>(),
        reloaded.subtotal
    );
    assert_eq!(
        reloaded.items.iter().map(|item| item.total).sum::<i64>(),
        reloaded.total
    );

    // Re-confirming is idempotent and must not duplicate the line.
    let again = service
        .confirm_order_edit(&order.id, &added.id)
        .await
        .unwrap();
    assert_eq!(again.status, OrderEditStatus::Confirmed);
    assert_eq!(service.get(&order.id).await.unwrap().items.len(), 2);

    let error = service
        .cancel_order_edit(&order.id, &added.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn order_edits_validate_before_quantities_and_overflow() {
    let service = service().await;
    let order = order(&service, 2).await;

    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: Some(order.items[0].id.clone()),
                    action: OrderEditAction::Remove,
                    product_id: None,
                    variant_id: None,
                    product_title: None,
                    variant_title: None,
                    sku: None,
                    unit_price: None,
                    before_quantity: Some(1),
                    after_quantity: 0,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let update = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: Some(order.items[0].id.clone()),
                    action: OrderEditAction::Update,
                    product_id: None,
                    variant_id: None,
                    product_title: None,
                    variant_title: None,
                    sku: None,
                    unit_price: None,
                    before_quantity: Some(2),
                    after_quantity: 1,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(update.after_total, 1_000);
    let confirmed = service
        .confirm_order_edit(&order.id, &update.id)
        .await
        .unwrap();
    assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
    assert_eq!(service.get(&order.id).await.unwrap().total, 1_000);
}

#[tokio::test]
async fn confirming_an_edit_keeps_all_order_totals_consistent() {
    let service = service().await;
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 2,
                subtotal: Some(2_000),
                discount_total: Some(200),
                tax_total: Some(180),
                total: Some(1_980),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 2_000,
            discount_total: 200,
            shipping_total: 500,
            tax_total: 180,
            tax_lines: vec![tax_line(180)],
            total: 2_480,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    assert_eq!(order.total, 2_480);

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("add gift".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: Some("GIFT-1".to_owned()),
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(edit.after_subtotal, 2_500);
    assert_eq!(edit.after_total, 2_980);

    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.after_subtotal, 2_500);
    assert_eq!(confirmed.after_total, 2_980);

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.discount_total, 200);
    assert_eq!(reloaded.shipping_total, 500);
    assert_eq!(reloaded.tax_total, 180);
    assert_eq!(reloaded.total, 2_980);
    assert_eq!(
        reloaded.total,
        reloaded.subtotal - reloaded.discount_total + reloaded.shipping_total + reloaded.tax_total
    );
    // The added line is materialized and every order total is the sum of
    // the materialized lines.
    assert_eq!(reloaded.items.len(), 2);
    let purchased = reloaded
        .items
        .iter()
        .find(|item| item.sku.is_none())
        .expect("the purchased line must survive the edit");
    assert_eq!(purchased.quantity, 2);
    assert_eq!(purchased.subtotal, 2_000);
    assert_eq!(purchased.discount_total, 200);
    assert_eq!(purchased.tax_total, 180);
    assert_eq!(purchased.total, 1_980);
    let added_line = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be materialized");
    assert_eq!(added_line.subtotal, 500);
    assert_eq!(added_line.total, 500);
    assert_eq!(
        reloaded.items.iter().map(|item| item.subtotal).sum::<i64>(),
        reloaded.subtotal
    );
    assert_eq!(
        reloaded
            .items
            .iter()
            .map(|item| item.discount_total)
            .sum::<i64>(),
        reloaded.discount_total
    );
    assert_eq!(
        reloaded
            .items
            .iter()
            .map(|item| item.tax_total)
            .sum::<i64>(),
        reloaded.tax_total
    );
    assert_eq!(
        reloaded.items.iter().map(|item| item.total).sum::<i64>() + reloaded.shipping_total,
        reloaded.total
    );
}
