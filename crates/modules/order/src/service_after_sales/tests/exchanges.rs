use super::*;

#[tokio::test]
async fn exchanges_track_inbound_and_outbound_items() {
    let service = service().await;
    let order = order(&service, 2).await;
    let variant = VariantId::new();

    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: Some("wrong size".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(variant.clone()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Requested);
    assert_eq!(exchange.inbound_items.len(), 1);
    assert_eq!(exchange.outbound_items.len(), 1);
    assert_eq!(
        exchange.outbound_items[0].replacement_variant_id.as_ref(),
        Some(&variant)
    );

    let error = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[])
        .await
        .unwrap();
    assert_eq!(accepted.status, ExchangeStatus::Accepted);
}

#[tokio::test]
async fn an_exchanged_inbound_unit_cannot_also_be_claimed() {
    let service = service().await;
    let order = order(&service, 1).await;
    service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let error = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    item_type: ClaimItemType::Refund,
                    replacement_variant_id: None,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
