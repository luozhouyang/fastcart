use super::*;

#[tokio::test]
async fn lists_payments_with_order_and_status_filters() {
    let (service, _db) = setup().await;
    service.register_provider(manual(Some("secret"))).unwrap();
    let mut first_input = input("manual", 500);
    first_input.order_id = Some(OrderId::new());
    let first = service.create_session(first_input).await.unwrap();
    service.capture(first.id.as_str()).await.unwrap();

    let mut second_input = input("manual", 700);
    second_input.order_id = Some(OrderId::new());
    let second = service.create_session(second_input).await.unwrap();
    service.capture(second.id.as_str()).await.unwrap();

    let by_order = service
        .list_payments(ListPaymentsFilter::new().with_order(first.order_id.clone().unwrap()))
        .await
        .unwrap();
    assert_eq!(by_order.count, 1);
    assert_eq!(by_order.data[0].amount, 500);

    let captured = service
        .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Captured))
        .await
        .unwrap();
    assert_eq!(captured.count, 2);

    let page = service
        .list_payments(ListPaymentsFilter {
            pagination: Pagination::new(Some(1), Some(1)).unwrap(),
            ..ListPaymentsFilter::new()
        })
        .await
        .unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.count, 2);
    assert_eq!(page.pagination.limit, 1);

    let empty: PageResult<crate::domain::Payment> = service
        .list_payments(ListPaymentsFilter::new().with_status(PaymentStatus::Refunded))
        .await
        .unwrap();
    assert!(empty.data.is_empty());
}
