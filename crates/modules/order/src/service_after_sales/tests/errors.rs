use super::*;

#[tokio::test]
async fn canceled_orders_reject_after_sales_flows() {
    let service = service().await;
    let order = order(&service, 1).await;
    let canceled = service.cancel(&order.id, None).await.unwrap();
    assert_eq!(canceled.status, OrderStatus::Canceled);

    let error = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn missing_order_or_line_is_not_found() {
    let service = service().await;
    let order = order(&service, 1).await;

    let missing_order = service
        .create_return(
            &OrderId::new(),
            CreateReturnInput {
                reason: None,
                refund_id: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(missing_order.is_not_found());

    let missing_line = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: None,
                refund_id: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: fastcart_kernel::LineItemId::new(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(missing_line.is_not_found());
}
