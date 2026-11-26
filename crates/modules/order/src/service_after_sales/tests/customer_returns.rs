use super::*;

use crate::after_sales::{CreateReturnInput, ReturnItemInput, ReturnSource, ReturnStatus};
use crate::domain::{CreateOrderInput, CreateOrderItemInput, FulfillmentInput, Order};
use fastcart_kernel::{CustomerId, Metadata};
use serde_json::json;

/// Creates an order that belongs to `customer_id` and, when `ship` is set,
/// confirms and fulfills it so the customer-facing return rule accepts it.
async fn owned_order(
    service: &OrderService,
    customer_id: &CustomerId,
    quantity: i64,
    ship: bool,
) -> Order {
    let unit_price = 1_000;
    let order = service
        .create_order(CreateOrderInput {
            customer_id: Some(customer_id.clone()),
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    if ship {
        service.confirm(&order.id).await.unwrap();
        service
            .fulfill(&order.id, FulfillmentInput::default())
            .await
            .unwrap();
    }
    order
}

fn customer_return_input(order: &Order, quantity: i64) -> CreateReturnInput {
    CreateReturnInput {
        reason: Some("wrong size".to_owned()),
        reason_id: None,
        refund_id: None,
        metadata: Metadata::empty(),
        items: vec![ReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity,
            reason: None,
        }],
    }
}

#[tokio::test]
async fn customer_return_is_marked_and_starts_requested() {
    let service = service().await;
    let customer_id = CustomerId::new();
    let order = owned_order(&service, &customer_id, 2, true).await;

    let created = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap();
    assert_eq!(created.source, ReturnSource::Customer);
    assert_eq!(created.status, ReturnStatus::Requested);
    assert!(created.requested_at.is_some());

    // The admin path keeps its own source and draft state.
    let admin_created = service
        .create_return(&order.id, customer_return_input(&order, 1))
        .await
        .unwrap();
    assert_eq!(admin_created.source, ReturnSource::Admin);
    assert_eq!(admin_created.status, ReturnStatus::Draft);
    assert!(admin_created.requested_at.is_none());
}

#[tokio::test]
async fn customer_return_rejects_unshipped_orders() {
    let service = service().await;
    let customer_id = CustomerId::new();
    let order = owned_order(&service, &customer_id, 2, false).await;

    let error = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap_err();
    match error {
        fastcart_kernel::AppError::Conflict(message) => {
            assert!(message.contains("pending"), "{message}");
            assert!(message.contains("not_fulfilled"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn customer_return_hides_orders_of_other_customers() {
    let service = service().await;
    let owner = CustomerId::new();
    let stranger = CustomerId::new();
    let order = owned_order(&service, &owner, 2, true).await;

    let error = service
        .create_customer_return(&stranger, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(error.is_not_found());

    let error = service
        .list_customer_returns(&stranger, &order.id)
        .await
        .unwrap_err();
    assert!(error.is_not_found());
}

#[tokio::test]
async fn customer_cannot_hold_two_open_returns() {
    let service = service().await;
    let customer_id = CustomerId::new();
    let order = owned_order(&service, &customer_id, 4, true).await;

    service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap();

    let error = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    // A terminal return no longer blocks a new request.
    let returns = service
        .list_customer_returns(&customer_id, &order.id)
        .await
        .unwrap();
    service
        .cancel_return(&order.id, &returns[0].id)
        .await
        .unwrap();
    let second = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 1))
        .await
        .unwrap();
    assert_eq!(second.status, ReturnStatus::Requested);
}

#[tokio::test]
async fn customer_return_quantity_is_checked_against_remaining() {
    let service = service().await;
    let customer_id = CustomerId::new();
    let order = owned_order(&service, &customer_id, 2, true).await;

    let error = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 3))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    // An admin return for part of the line reduces what the customer can take.
    service
        .create_return(&order.id, customer_return_input(&order, 1))
        .await
        .unwrap();
    let error = service
        .create_customer_return(&customer_id, &order.id, customer_return_input(&order, 2))
        .await
        .unwrap_err();
    match error {
        fastcart_kernel::AppError::Conflict(message) => {
            assert!(message.contains("only 1 remain available"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}
