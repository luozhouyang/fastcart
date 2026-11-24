use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{
    CustomerId, OrderId, Pagination, PaymentId, ProductId, StoreId, VariantId, now_timestamp,
};
use serde_json::json;

use super::OrderService;
use crate::domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentStatus, OrderFilter, OrderStatus, PaymentStatus, RefundInput, RefundStatus,
};

async fn service() -> OrderService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    OrderService::new(db)
}

fn item(unit_price: i64, quantity: i64) -> CreateOrderItemInput {
    CreateOrderItemInput {
        product_title: "Widget".to_owned(),
        unit_price,
        quantity,
        requires_shipping: true,
        ..CreateOrderItemInput::default()
    }
}

fn order_input(items: Vec<CreateOrderItemInput>) -> CreateOrderInput {
    let subtotal: i64 = items
        .iter()
        .map(|item| item.unit_price * item.quantity)
        .sum();
    CreateOrderInput {
        email: "buyer@example.com".to_owned(),
        currency_code: "USD".to_owned(),
        items,
        subtotal,
        total: subtotal,
        shipping_address: json!({"city": "Berlin"}),
        billing_address: json!({"city": "Berlin"}),
        ..CreateOrderInput::default()
    }
}

#[tokio::test]
async fn creates_order_with_immutable_snapshots() {
    let service = service().await;
    let mut line = item(1_250, 2);
    line.product_id = Some(ProductId::new());
    line.variant_id = Some(VariantId::new());
    line.product_title = "Original title".to_owned();
    line.variant_title = Some("Large".to_owned());
    line.sku = Some("SKU-1".to_owned());
    line.thumbnail = Some("https://example.com/a.png".to_owned());
    line.is_tax_inclusive = true;
    line.requires_shipping = false;
    line.metadata.insert("color", json!("red"));
    line.adjustments = vec![CreateAdjustmentInput {
        code: Some("SAVE".to_owned()),
        amount: -100,
        ..CreateAdjustmentInput::default()
    }];

    let order = service.create_order(order_input(vec![line])).await.unwrap();

    assert_eq!(order.status, OrderStatus::Pending);
    assert_eq!(order.payment_status, PaymentStatus::Awaiting);
    assert_eq!(order.fulfillment_status, FulfillmentStatus::NotFulfilled);
    assert_eq!(order.refund_status, RefundStatus::NotRefunded);
    assert_eq!(order.display_number, 1);
    assert_eq!(order.subtotal, 2_500);
    assert_eq!(order.total, 2_500);
    assert_eq!(order.items.len(), 1);
    let snapshot = &order.items[0];
    assert_eq!(snapshot.product_title, "Original title");
    assert_eq!(snapshot.variant_title.as_deref(), Some("Large"));
    assert_eq!(snapshot.sku.as_deref(), Some("SKU-1"));
    assert_eq!(snapshot.unit_price, 1_250);
    assert_eq!(snapshot.quantity, 2);
    assert!(snapshot.is_tax_inclusive);
    assert!(!snapshot.requires_shipping);
    assert_eq!(snapshot.adjustments.len(), 1);
    assert_eq!(order.status_history.len(), 1);
}

#[tokio::test]
async fn create_is_idempotent_and_writes_nothing_extra() {
    let service = service().await;
    let mut input = order_input(vec![item(1_000, 1)]);
    input.idempotency_key = Some("order-key".to_owned());

    let first = service.create_order(input.clone()).await.unwrap();
    let second = service.create_order(input).await.unwrap();

    assert_eq!(first.id, second.id);
    assert_eq!(second.items.len(), 1);
    let page = service
        .list(OrderFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 1);
}

#[tokio::test]
async fn display_numbers_increment() {
    let service = service().await;
    let first = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();
    let second = service
        .create_order(order_input(vec![item(200, 1)]))
        .await
        .unwrap();

    assert_eq!(first.display_number, 1);
    assert_eq!(second.display_number, 2);
    let found = service.get_by_display_number(2).await.unwrap();
    assert_eq!(found.id, second.id);
    assert!(service.get_by_display_number(99).await.is_err());
}

#[tokio::test]
async fn lists_orders_with_filters_and_pagination() {
    let service = service().await;
    let customer = CustomerId::new();

    let mut first_input = order_input(vec![item(100, 1)]);
    first_input.email = "first@example.com".to_owned();
    first_input.customer_id = Some(customer.clone());
    let first = service.create_order(first_input).await.unwrap();

    let mut second_input = order_input(vec![item(200, 1)]);
    second_input.email = "second@example.com".to_owned();
    service.create_order(second_input).await.unwrap();

    let mut third_input = order_input(vec![item(300, 1)]);
    third_input.email = "third@example.com".to_owned();
    service.create_order(third_input).await.unwrap();

    let all = service
        .list(OrderFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(all.count, 3);
    assert_eq!(all.data.len(), 3);
    for order in &all.data {
        assert!(!order.items.is_empty(), "order {} lost its items", order.id);
        assert!(
            order.items.iter().all(|item| item.order_id == order.id),
            "items leaked across orders in the batched list loader"
        );
        assert!(order.status_history.iter().all(|h| h.order_id == order.id));
        assert!(
            order
                .fulfillments
                .iter()
                .all(|fulfillment| fulfillment.order_id == order.id)
        );
        assert!(
            order
                .refunds
                .iter()
                .all(|refund| refund.order_id == order.id)
        );
    }

    let filtered = service
        .list(
            OrderFilter {
                email: Some("FIRST@example.com".to_owned()),
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(filtered.count, 1);
    assert_eq!(filtered.data[0].id, first.id);

    let by_customer = service
        .list(
            OrderFilter {
                customer_id: Some(customer),
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_customer.count, 1);

    let page = service
        .list(
            OrderFilter::default(),
            Pagination {
                limit: 1,
                offset: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.count, 3);
    assert_eq!(page.data.len(), 1);

    let future = now_timestamp() + 3_600;
    let none = service
        .list(
            OrderFilter {
                from: Some(future),
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(none.count, 0);
}

#[tokio::test]
async fn filters_orders_by_status() {
    let service = service().await;
    let first = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();
    service.confirm(&first.id).await.unwrap();
    service
        .create_order(order_input(vec![item(200, 1)]))
        .await
        .unwrap();

    let pending = service
        .list(
            OrderFilter {
                status: Some(OrderStatus::Pending),
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(pending.count, 1);

    let confirmed = service
        .list(
            OrderFilter {
                status: Some(OrderStatus::Confirmed),
                payment_status: Some(PaymentStatus::Awaiting),
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(confirmed.count, 1);
    assert_eq!(confirmed.data[0].id, first.id);
}

#[tokio::test]
async fn confirm_and_start_processing_follow_the_lifecycle() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();

    let confirmed = service.confirm(&order.id).await.unwrap();
    assert_eq!(confirmed.status, OrderStatus::Confirmed);
    let processing = service.start_processing(&order.id).await.unwrap();
    assert_eq!(processing.status, OrderStatus::Processing);

    let error = service.confirm(&order.id).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn audit_commits_with_the_order_mutation_and_skips_repeats() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));
    let order = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();

    service.confirm(&order.id).await.unwrap();

    let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "order.confirmed");
    assert_eq!(audits[0].resource_type.as_deref(), Some("order"));
    assert_eq!(audits[0].resource_id.as_deref(), Some(order.id.as_str()));
    assert_eq!(audits[0].actor_id.as_deref(), Some("user_1"));
    assert_eq!(audits[0].store_id.as_deref(), Some(db.store_id().as_str()));

    // Repeating the transition must not append a second audit row.
    assert!(service.confirm(&order.id).await.is_err());
    assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn cancel_records_history_and_is_idempotent_safe() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();

    let canceled = service
        .cancel(&order.id, Some("customer request"))
        .await
        .unwrap();
    assert_eq!(canceled.status, OrderStatus::Canceled);
    assert!(canceled.canceled_at.is_some());
    let history = service.list_history(&order.id).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].note.as_deref(), Some("customer request"));

    let error = service.cancel(&order.id, None).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn full_fulfillment_records_a_fulfillment() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(100, 2)]))
        .await
        .unwrap();
    let order = service.confirm(&order.id).await.unwrap();

    let fulfilled = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_id: None,
                provider_fulfillment_id: None,
                provider_data: json!({}),
                items: Vec::new(),
                tracking_numbers: vec!["TRACK-1".to_owned()],
                shipped_at: None,
                partial: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(fulfilled.fulfillment_status, FulfillmentStatus::Fulfilled);
    assert_eq!(fulfilled.fulfillments.len(), 1);
    assert_eq!(fulfilled.fulfillments[0].items.len(), 1);
    assert_eq!(fulfilled.fulfillments[0].items[0].quantity, 2);
    assert_eq!(fulfilled.fulfillments[0].provider_id, "manual");
    assert_eq!(
        fulfilled.fulfillments[0].tracking_numbers,
        vec!["TRACK-1".to_owned()]
    );
    assert!(fulfilled.fulfillments[0].shipped_at.is_some());
}

#[tokio::test]
async fn partial_fulfillment_is_tracked_separately() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(100, 3)]))
        .await
        .unwrap();
    let order = service.confirm(&order.id).await.unwrap();

    let partial = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_id: None,
                provider_fulfillment_id: None,
                provider_data: json!({}),
                items: vec![crate::domain::FulfillmentItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
                tracking_numbers: vec!["TRACK-2".to_owned()],
                shipped_at: Some(1_700_000_000),
                partial: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        partial.fulfillment_status,
        FulfillmentStatus::PartiallyFulfilled
    );
    assert_eq!(partial.fulfillments[0].shipped_at, Some(1_700_000_000));
    assert_eq!(partial.fulfillments[0].items[0].quantity, 1);

    let full = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_id: None,
                provider_fulfillment_id: None,
                provider_data: json!({}),
                items: Vec::new(),
                tracking_numbers: vec!["TRACK-3".to_owned()],
                shipped_at: None,
                partial: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(full.fulfillment_status, FulfillmentStatus::Fulfilled);
    assert_eq!(full.fulfillments.len(), 2);
}

#[tokio::test]
async fn refunds_progress_from_partial_to_full() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1_000, 2)]))
        .await
        .unwrap();
    let payment = PaymentId::new();

    let partial = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: payment.clone(),
                amount: 500,
                currency: "USD".to_owned(),
                reason: Some("damaged".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(partial.refund_status, RefundStatus::PartiallyRefunded);
    assert_eq!(partial.payment_status, PaymentStatus::PartiallyRefunded);
    assert_eq!(partial.refunds.len(), 1);

    let full = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: payment,
                amount: 1_500,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(full.refund_status, RefundStatus::Refunded);
    assert_eq!(full.payment_status, PaymentStatus::Refunded);
    assert_eq!(full.refunds.len(), 2);
}

#[tokio::test]
async fn failed_refund_reverts_payment_status_and_is_idempotent() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1_000, 2)]))
        .await
        .unwrap();
    let payment = PaymentId::new();
    let refunded = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: payment.clone(),
                amount: 500,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(refunded.payment_status, PaymentStatus::PartiallyRefunded);

    assert!(
        service
            .mark_payment_refund_failed(&order.id, payment.as_str(), 500)
            .await
            .unwrap()
    );
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.payment_status, PaymentStatus::Captured);
    assert_eq!(order.refunds.len(), 1);
    assert_eq!(order.refunds[0].status, "failed");

    // Replaying the provider failure is an idempotent no-op.
    assert!(
        !service
            .mark_payment_refund_failed(&order.id, payment.as_str(), 500)
            .await
            .unwrap()
    );
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.payment_status, PaymentStatus::Captured);
    assert_eq!(order.refunds[0].status, "failed");
}

#[tokio::test]
async fn failed_full_refund_recomputes_payment_status_from_remaining_refunds() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1_000, 2)]))
        .await
        .unwrap();
    let payment = PaymentId::new();
    for amount in [500, 1_500] {
        service
            .add_refund(
                &order.id,
                RefundInput {
                    payment_id: payment.clone(),
                    amount,
                    currency: "USD".to_owned(),
                    reason: None,
                },
            )
            .await
            .unwrap();
    }
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.payment_status, PaymentStatus::Refunded);

    assert!(
        service
            .mark_payment_refund_failed(&order.id, payment.as_str(), 1_500)
            .await
            .unwrap()
    );
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.payment_status, PaymentStatus::PartiallyRefunded);
    assert_eq!(
        order
            .refunds
            .iter()
            .filter(|refund| refund.status == "failed")
            .count(),
        1
    );

    assert!(
        service
            .mark_payment_refund_failed(&order.id, payment.as_str(), 500)
            .await
            .unwrap()
    );
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.payment_status, PaymentStatus::Captured);
    assert!(order.refunds.iter().all(|refund| refund.status == "failed"));
    assert!(
        !service
            .mark_payment_refund_failed(&order.id, payment.as_str(), 500)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn rejects_over_refunds() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1_000, 1)]))
        .await
        .unwrap();

    service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: 600,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap();

    let error = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: 500,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_currency_email_and_amounts() {
    let service = service().await;

    let mut input = order_input(vec![item(100, 1)]);
    input.currency_code = "US".to_owned();
    assert!(service.create_order(input).await.is_err());

    let mut input = order_input(vec![item(100, 1)]);
    input.email = "bad".to_owned();
    assert!(service.create_order(input).await.is_err());

    let mut input = order_input(vec![item(-100, 1)]);
    input.subtotal = -100;
    input.total = -100;
    assert!(service.create_order(input).await.is_err());

    let mut input = order_input(vec![item(100, 0)]);
    input.subtotal = 0;
    input.total = 0;
    assert!(service.create_order(input).await.is_err());

    let mut input = order_input(vec![item(100, 1)]);
    input.total = 999;
    assert!(service.create_order(input).await.is_err());

    let input = order_input(Vec::new());
    assert!(service.create_order(input).await.is_err());
}

#[tokio::test]
async fn rejects_invalid_refund_amounts_and_currencies() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1_000, 1)]))
        .await
        .unwrap();

    let zero = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: 0,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(zero, fastcart_kernel::AppError::Validation(_)));

    let wrong_currency = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: 100,
                currency: "EUR".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        wrong_currency,
        fastcart_kernel::AppError::Validation(_)
    ));
}

#[tokio::test]
async fn order_level_adjustments_must_be_in_the_total() {
    let service = service().await;
    let mut input = order_input(vec![item(1_000, 1)]);
    input.adjustments = vec![CreateAdjustmentInput {
        code: Some("GIFT".to_owned()),
        amount: 250,
        ..CreateAdjustmentInput::default()
    }];
    let error = service.create_order(input).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let mut input = order_input(vec![item(1_000, 1)]);
    input.adjustments = vec![CreateAdjustmentInput {
        code: Some("GIFT".to_owned()),
        amount: 250,
        ..CreateAdjustmentInput::default()
    }];
    input.total = 1_250;
    let order = service.create_order(input).await.unwrap();
    assert_eq!(order.total, 1_250);
    assert_eq!(order.adjustments.len(), 1);
}

#[tokio::test]
async fn records_and_lists_history() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(100, 1)]))
        .await
        .unwrap();
    service
        .record_history(
            &order.id,
            OrderStatus::Pending,
            Some(OrderStatus::Pending),
            Some("manual note"),
        )
        .await
        .unwrap();

    let history = service.list_history(&order.id).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].note.as_deref(), Some("manual note"));
}

#[tokio::test]
async fn missing_order_is_not_found() {
    let service = service().await;
    assert!(
        service
            .get(&OrderId::new())
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .list(OrderFilter::default(), Pagination::default())
            .await
            .unwrap()
            .data
            .is_empty()
    );
}
#[tokio::test]
async fn reconciles_provider_refunds_missing_from_the_order_ledger() {
    let service = service().await;
    let order = service
        .create_order(order_input(vec![item(1000, 1)]))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO refunds \
             (id, payment_id, order_id, amount, currency_code, status, provider_refund_id, \
              reason, created_at, updated_at) \
             VALUES ('refund_provider', NULL, ?, 400, 'USD', 'succeeded', 'ch_123', NULL, 0, 0)",
    )
    .bind(order.id.as_str())
    .execute(service.db.pool())
    .await
    .unwrap();

    assert_eq!(service.reconcile_provider_refunds().await.unwrap(), 1);
    let refreshed = service.get(&order.id).await.unwrap();
    assert_eq!(refreshed.refunds.len(), 1);
    assert_eq!(refreshed.refunds[0].amount, 400);
    assert_eq!(refreshed.refunds[0].status, "succeeded");
    assert_eq!(refreshed.payment_status, PaymentStatus::PartiallyRefunded);
    assert_eq!(refreshed.refund_status, RefundStatus::PartiallyRefunded);

    // Replaying the reconciliation does not duplicate the local row.
    assert_eq!(service.reconcile_provider_refunds().await.unwrap(), 0);
}
