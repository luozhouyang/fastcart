use super::support::*;
use super::*;

#[tokio::test]
async fn return_refunds_move_money_once_and_link_the_order_refund() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    let first = fixture
        .workflow
        .refund_return(
            &order.id,
            &order_return.id,
            Some("customer return".to_owned()),
        )
        .await
        .unwrap();

    let provider_refund = first.provider_refund.as_ref().unwrap();
    assert_eq!(provider_refund.amount, 2_000);
    assert_eq!(provider_refund.status, PaymentRefundStatus::Succeeded);
    assert_eq!(first.order.refunds.len(), 1);
    assert_eq!(first.order.refund_status, OrderRefundStatus::Refunded);
    let order_refund = first.order_refund.as_ref().unwrap();
    assert_eq!(order_refund.amount, 2_000);
    assert_eq!(order_refund.payment_id.as_ref(), Some(&payment.id));
    assert_eq!(order_return.refund_id, None);
    assert_eq!(
        first.order_return.refund_id.as_ref(),
        Some(&order_refund.id)
    );

    // Replaying the same call must not create another provider or order refund.
    let replay = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap();
    assert_eq!(replay.order.refunds.len(), 1);
    assert_eq!(replay.order_refund.as_ref().unwrap().id, order_refund.id);
    assert_eq!(
        replay.provider_refund.as_ref().unwrap().id,
        provider_refund.id
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
async fn retry_after_a_partial_failure_relinks_without_refunding() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    // Simulate a crash after the provider and order refunds were written but
    // before the return link was attached.
    let key = format!("return-refund:{}", order_return.id.as_str());
    let provider_refund = fixture
        .payments
        .refund_with_idempotency(payment.id.as_str(), 2_000, None, Some(key.clone()))
        .await
        .unwrap();
    let (_, order_refund_id) = fixture
        .orders
        .add_refund_idempotent(
            &order.id,
            RefundInput {
                payment_id: payment.id.clone(),
                amount: 2_000,
                currency: "USD".to_owned(),
                reason: None,
            },
            &format!("after-sales:{key}"),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .orders
            .get_return(&order.id, &order_return.id)
            .await
            .unwrap()
            .refund_id
            .is_none()
    );

    let result = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap();
    assert_eq!(
        result.order_return.refund_id.as_ref(),
        Some(&order_refund_id)
    );
    assert_eq!(result.order.refunds.len(), 1);
    assert_eq!(
        result.provider_refund.as_ref().unwrap().id,
        provider_refund.id
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
async fn partial_return_refunds_the_proportional_line_total() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 1).await;
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    let result = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap();

    assert_eq!(result.provider_refund.as_ref().unwrap().amount, 1_000);
    assert_eq!(result.order_refund.as_ref().unwrap().amount, 1_000);
    assert_eq!(
        result.order.refund_status,
        OrderRefundStatus::PartiallyRefunded
    );
    assert_eq!(
        fixture
            .payments
            .list_refunds(payment.id.as_str())
            .await
            .unwrap()[0]
            .amount,
        1_000
    );
}

#[tokio::test]
async fn claim_refunds_only_refund_type_items_once() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: Some("damaged".to_owned()),
                reason_id: None,
                metadata: Metadata::empty(),
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

    let result = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, None)
        .await
        .unwrap();
    assert_eq!(result.claim.status, ClaimStatus::Accepted);
    assert_eq!(result.provider_refund.as_ref().unwrap().amount, 1_000);
    assert_eq!(result.order_refund.as_ref().unwrap().amount, 1_000);
    assert_eq!(
        result.claim.refund_id.as_ref(),
        Some(&result.order_refund.as_ref().unwrap().id)
    );

    let replay = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, Some("retry".to_owned()))
        .await
        .unwrap();
    assert_eq!(replay.order.refunds.len(), 1);
    assert_eq!(
        replay.order_refund.as_ref().unwrap().id,
        result.order_refund.as_ref().unwrap().id
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
async fn claims_without_refund_items_do_not_refund() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
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
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let result = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, None)
        .await
        .unwrap();
    assert_eq!(result.claim.status, ClaimStatus::Accepted);
    assert!(result.claim.refund_id.is_none());
    assert!(result.order_refund.is_none());
    assert!(result.provider_refund.is_none());
    assert!(result.order.refunds.is_empty());
}

#[tokio::test]
async fn missing_captured_payment_is_a_conflict() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 1, None, true).await;
    let order_return = requested_return(&fixture.orders, &order, 1).await;
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    let error = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn refunding_an_unreceived_return_is_rejected() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 1, None, true).await;
    capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 1).await;

    let error = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
    assert!(
        fixture
            .orders
            .get_return(&order.id, &order_return.id)
            .await
            .unwrap()
            .refund_id
            .is_none()
    );
}

#[tokio::test]
async fn written_off_units_are_excluded_from_the_refund() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    fixture
        .orders
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    let written_off = fixture
        .orders
        .write_off_return(
            &order.id,
            &order_return.id,
            fastcart_module_order::WriteOffReturnInput {
                items: vec![fastcart_module_order::WriteOffReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(written_off.status, ReturnStatus::Received);
    assert_eq!(written_off.items[0].received_quantity, 2);
    assert_eq!(written_off.items[0].written_off_quantity, 1);

    let result = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap();
    // One of the two received units was written off, so only one is refunded.
    assert_eq!(result.provider_refund.as_ref().unwrap().amount, 1_000);
    assert_eq!(result.order_refund.as_ref().unwrap().amount, 1_000);

    let replay = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap();
    assert_eq!(
        replay.order_refund.as_ref().unwrap().id,
        result.order_refund.as_ref().unwrap().id
    );
}
