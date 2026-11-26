use super::support::*;
use super::*;

#[tokio::test]
async fn claim_replacements_are_fulfilled_and_consume_stock_once() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 1, Some(original.clone()), true).await;
    let (inventory, _, location) = stock_order(&fixture.db, &order, &original, 10, 1).await;
    let replacement_item =
        stock_replacement_variant(&inventory, &order.id, &replacement, &location, 5).await;
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        4
    );

    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: Some("wrong size".to_owned()),
                reason_id: None,
                metadata: Metadata::empty(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Replace,
                    replacement_variant_id: Some(replacement.clone()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let first = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "manual", vec!["TRACK-1".to_owned()])
        .await
        .unwrap();
    assert_eq!(first.claim.status, ClaimStatus::Accepted);
    assert_eq!(first.consumed_units, 1);
    let fulfillment = first.fulfillment.as_ref().unwrap();
    assert_eq!(fulfillment.provider_id, "manual");
    assert_eq!(fulfillment.tracking_numbers, vec!["TRACK-1".to_owned()]);
    assert_eq!(fulfillment.items.len(), 1);
    assert_eq!(fulfillment.items[0].variant_id, replacement);
    assert_eq!(fulfillment.items[0].quantity, 1);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        3
    );

    let replay = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "manual", Vec::new())
        .await
        .unwrap();
    assert_eq!(replay.consumed_units, 0);
    assert_eq!(replay.fulfillment.as_ref().unwrap().id, fulfillment.id);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        3
    );
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

#[tokio::test]
async fn refund_and_replacement_claim_lines_compose() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_two_line_order(&fixture.orders).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let (inventory, _, location) = stock_order(&fixture.db, &order, &original, 10, 1).await;
    let replacement_item =
        stock_replacement_variant(&inventory, &order.id, &replacement, &location, 5).await;

    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                items: vec![
                    ClaimItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 1,
                        item_type: ClaimItemType::Refund,
                        replacement_variant_id: None,
                        reason: None,
                    },
                    ClaimItemInput {
                        order_item_id: order.items[1].id.clone(),
                        quantity: 1,
                        item_type: ClaimItemType::Replace,
                        replacement_variant_id: Some(replacement.clone()),
                        reason: None,
                    },
                ],
            },
        )
        .await
        .unwrap();

    let refund = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, None)
        .await
        .unwrap();
    // Only the refund line is paid out.
    assert_eq!(refund.provider_refund.as_ref().unwrap().amount, 1_000);
    assert!(refund.claim.refund_id.is_some());

    let fulfillment = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "manual", Vec::new())
        .await
        .unwrap();
    // Only the replace line is shipped.
    assert_eq!(fulfillment.consumed_units, 1);
    assert_eq!(fulfillment.fulfillment.as_ref().unwrap().items.len(), 1);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        fixture
            .payments
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn claim_fulfillment_does_not_call_the_provider_without_stock() {
    let provider = Arc::new(TestProvider::new("test"));
    let shipping = FulfillmentProviderRegistry::new();
    shipping.register(provider.clone()).unwrap();
    let fixture = fixture_with_shipping(shipping).await;

    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 1, Some(original.clone()), true).await;
    let _ = stock_order(&fixture.db, &order, &original, 10, 1).await;
    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Replace,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let error = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "test", Vec::new())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert_eq!(
        provider.calls(),
        0,
        "the provider must not be called when stock consumption fails"
    );
}

#[tokio::test]
async fn claim_fulfillment_restores_stock_when_the_provider_fails() {
    let provider = Arc::new(TestProvider::new("test"));
    provider.set_failing(true);
    let shipping = FulfillmentProviderRegistry::new();
    shipping.register(provider.clone()).unwrap();
    let fixture = fixture_with_shipping(shipping).await;

    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 1, Some(original.clone()), true).await;
    let (inventory, _, location) = stock_order(&fixture.db, &order, &original, 10, 1).await;
    let replacement_item =
        stock_replacement_variant(&inventory, &order.id, &replacement, &location, 5).await;
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        4
    );
    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Replace,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let error = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "test", Vec::new())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::ExternalService(_)));
    assert_eq!(provider.calls(), 1);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        4,
        "a provider failure must put the consumed replacements back"
    );
    assert_eq!(
        inventory
            .list_movements(&replacement_item)
            .await
            .unwrap()
            .iter()
            .filter(|movement| movement.reason == "after_sales_consume")
            .count(),
        0
    );

    // A retry after the provider recovers consumes the stock once and records
    // the fulfillment.
    provider.set_failing(false);
    let fulfilled = fixture
        .workflow
        .fulfill_claim(&order.id, &claim.id, "test", Vec::new())
        .await
        .unwrap();
    assert_eq!(fulfilled.consumed_units, 1);
    assert_eq!(provider.calls(), 2);
    assert_eq!(
        inventory
            .available_quantity(&replacement_item, &location)
            .await
            .unwrap(),
        3
    );
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
