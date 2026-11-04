//! After-sales workflow: return/claim refunds and idempotent restocking.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    CheckoutId, InventoryItemId, InventoryLocationId, Metadata, StoreId, VariantId,
};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, ReservationLine,
    ReservationReference,
};
use fastcart_module_order::{
    ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateOrderInput,
    CreateOrderItemInput, CreateReturnInput, Order, OrderReturn, OrderService, ReceiveReturnInput,
    ReceiveReturnItemInput, RefundInput, RefundStatus as OrderRefundStatus, ReturnItemInput,
    ReturnStatus,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, Payment, PaymentService, ProviderRegistry,
    RefundStatus as PaymentRefundStatus,
};
use fastcart_workflows::AfterSalesWorkflow;
use serde_json::json;

struct Fixture {
    db: StoreDatabase,
    workflow: AfterSalesWorkflow,
    orders: OrderService,
    payments: PaymentService,
}

async fn fixture() -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let workflow = AfterSalesWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers.clone()),
    );
    Fixture {
        orders: OrderService::new(db.clone()),
        payments: PaymentService::with_registry(db.clone(), providers),
        db,
        workflow,
    }
}

async fn create_order(
    orders: &OrderService,
    quantity: i64,
    variant_id: Option<VariantId>,
    requires_shipping: bool,
) -> Order {
    orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity,
                variant_id,
                requires_shipping,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000 * quantity,
            total: 1_000 * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

async fn capture_payment(payments: &PaymentService, order: &Order) -> Payment {
    let session = payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: order.currency_code.clone(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    payments.capture(session.id.as_str()).await.unwrap()
}

async fn requested_return(orders: &OrderService, order: &Order, quantity: i64) -> OrderReturn {
    let created = orders
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: Some("damaged".to_owned()),
                refund_id: None,
                metadata: Metadata::empty(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    orders.request_return(&order.id, &created.id).await.unwrap()
}

async fn stock_order(
    db: &StoreDatabase,
    order: &Order,
    variant: &VariantId,
    stocked: i64,
    reserved: i64,
) -> (InventoryService, InventoryItemId, InventoryLocationId) {
    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main warehouse".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, stocked, "receiving", None)
        .await
        .unwrap();
    let checkout = CheckoutId::new();
    inventory
        .reserve(
            vec![ReservationLine::new(variant.clone(), reserved)],
            ReservationReference::new(checkout.clone()),
            60,
        )
        .await
        .unwrap();
    inventory.commit(&checkout, &order.id).await.unwrap();
    (inventory, item.id, location.id)
}

fn restock_movements(
    movements: &[fastcart_module_inventory::InventoryMovement],
) -> Vec<&fastcart_module_inventory::InventoryMovement> {
    movements
        .iter()
        .filter(|movement| movement.reason == "return_restock")
        .collect()
}

#[tokio::test]
async fn return_refunds_move_money_once_and_link_the_order_refund() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 2, None, true).await;
    let payment = capture_payment(&fixture.payments, &order).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;

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
async fn receiving_a_return_restocks_exactly_once() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        8
    );
    let order_return = requested_return(&fixture.orders, &order, 2).await;

    let first = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(first.order_return.status, ReturnStatus::Received);
    assert_eq!(first.restocked_units, 2);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    let restocks = restock_movements(&movements);
    assert_eq!(restocks.len(), 1);
    assert_eq!(restocks[0].location_id, location_id);
    assert_eq!(restocks[0].quantity_delta, 2);

    // Repeating the receive is a no-op for inventory.
    let replay = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(replay.order_return.status, ReturnStatus::Received);
    assert_eq!(replay.restocked_units, 0);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 1);
}

#[tokio::test]
async fn partial_receipts_restock_each_new_quantity() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;
    let line = order.items[0].id.clone();

    let first = fixture
        .workflow
        .receive_return(
            &order.id,
            &order_return.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: line.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(first.order_return.status, ReturnStatus::PartiallyReceived);
    assert_eq!(first.restocked_units, 1);

    let second = fixture
        .workflow
        .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(second.order_return.status, ReturnStatus::Received);
    assert_eq!(second.restocked_units, 1);
    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 2);
}

#[tokio::test]
async fn concurrent_receives_never_double_restock() {
    let fixture = fixture().await;
    let variant = VariantId::new();
    let order = create_order(&fixture.orders, 2, Some(variant.clone()), true).await;
    let (inventory, item_id, location_id) = stock_order(&fixture.db, &order, &variant, 10, 2).await;
    let order_return = requested_return(&fixture.orders, &order, 2).await;

    let (first, second) = tokio::join!(
        fixture
            .workflow
            .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default()),
        fixture
            .workflow
            .receive_return(&order.id, &order_return.id, ReceiveReturnInput::default()),
    );
    let restocked = first.as_ref().map_or(0, |result| result.restocked_units)
        + second.as_ref().map_or(0, |result| result.restocked_units);
    assert_eq!(restocked, 2);

    assert_eq!(
        inventory
            .available_quantity(&item_id, &location_id)
            .await
            .unwrap(),
        10
    );
    let movements = inventory.list_movements(&item_id).await.unwrap();
    assert_eq!(restock_movements(&movements).len(), 1);
}

#[tokio::test]
async fn missing_captured_payment_is_a_conflict() {
    let fixture = fixture().await;
    let order = create_order(&fixture.orders, 1, None, true).await;
    let order_return = requested_return(&fixture.orders, &order, 1).await;

    let error = fixture
        .workflow
        .refund_return(&order.id, &order_return.id, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
