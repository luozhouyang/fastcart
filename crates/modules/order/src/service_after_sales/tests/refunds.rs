use super::*;

#[tokio::test]
async fn idempotent_order_refunds_do_not_write_twice() {
    let service = service().await;
    let order = order(&service, 2).await;
    let input = RefundInput {
        payment_id: PaymentId::new(),
        amount: 500,
        currency: "USD".to_owned(),
        reason: Some("return".to_owned()),
    };

    let (first_order, first_id) = service
        .add_refund_idempotent(&order.id, input.clone(), "after-sales:return-refund:ret_1")
        .await
        .unwrap();
    let (second_order, second_id) = service
        .add_refund_idempotent(&order.id, input, "after-sales:return-refund:ret_1")
        .await
        .unwrap();

    assert_eq!(first_id, second_id);
    assert_eq!(first_order.refunds.len(), 1);
    assert_eq!(second_order.refunds.len(), 1);
    assert_eq!(second_order.refunds[0].id, first_id);
    assert_eq!(second_order.refunds[0].amount, 500);
}

#[tokio::test]
async fn after_sales_refund_links_are_idempotent() {
    let service = service().await;
    let order = order(&service, 2).await;

    let created = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    let refund = RefundId::new();
    let linked = service
        .attach_return_refund(&order.id, &created.id, &refund)
        .await
        .unwrap();
    assert_eq!(linked.refund_id.as_ref(), Some(&refund));
    let linked_again = service
        .attach_return_refund(&order.id, &created.id, &refund)
        .await
        .unwrap();
    assert_eq!(linked_again.refund_id.as_ref(), Some(&refund));
    let other = RefundId::new();
    assert!(matches!(
        service
            .attach_return_refund(&order.id, &created.id, &other)
            .await,
        Err(fastcart_kernel::AppError::Conflict(_))
    ));

    let claim = service
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
        .unwrap();
    let claim_refund = RefundId::new();
    let linked_claim = service
        .attach_claim_refund(&order.id, &claim.id, &claim_refund)
        .await
        .unwrap();
    assert_eq!(linked_claim.refund_id.as_ref(), Some(&claim_refund));
    let linked_claim_again = service
        .attach_claim_refund(&order.id, &claim.id, &claim_refund)
        .await
        .unwrap();
    assert_eq!(linked_claim_again.refund_id.as_ref(), Some(&claim_refund));
}
