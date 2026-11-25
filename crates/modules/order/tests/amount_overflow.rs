//! Overflow boundaries on the order side.
//!
//! The order service validates client-supplied totals by recomputing them, and
//! every arithmetic step in that recomputation is a `checked_*` call. These
//! tests drive each of those steps to the top of the `i64` range and assert the
//! documented `VALIDATION_ERROR` instead of a panic (debug) or a silent wrap
//! (release).

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, PaymentId, StoreId};
use fastcart_module_order::{
    CreateOrderEditInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput, Order,
    OrderEditAction, OrderEditItemInput, OrderService, RefundInput,
};

const MAX: i64 = i64::MAX;

async fn service() -> (StoreDatabase, OrderService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone());
    (db, service)
}

fn item(unit_price: i64, quantity: i64) -> CreateOrderItemInput {
    CreateOrderItemInput {
        product_title: "Widget".to_owned(),
        unit_price,
        quantity,
        ..CreateOrderItemInput::default()
    }
}

/// Places an order with exactly one line and no discounts or taxes.
async fn order_with_line(
    service: &OrderService,
    unit_price: i64,
    quantity: i64,
    shipping_total: i64,
) -> Order {
    let subtotal = unit_price.checked_mul(quantity).unwrap();
    let total = subtotal + shipping_total;
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![item(unit_price, quantity)],
            subtotal,
            shipping_total,
            total,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

fn assert_validation(error: &AppError, needle: &str) {
    assert!(
        matches!(error, AppError::Validation(message) if message.contains(needle)),
        "expected a validation error mentioning {needle:?}, got {error:?}"
    );
}

#[tokio::test]
async fn a_line_whose_subtotal_overflows_is_rejected() {
    let (_, service) = service().await;

    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![item(MAX, 2)],
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    // Unchecked arithmetic would panic here in a debug build and persist a
    // negative line subtotal in a release build.
    assert_eq!(MAX.wrapping_mul(2), -2);
    assert_validation(&error, "item subtotal overflow");
}

#[tokio::test]
async fn item_subtotals_that_overflow_the_order_are_rejected() {
    let (_, service) = service().await;

    // Each line is representable on its own; their sum is not.
    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![item(MAX, 1), item(MAX, 1)],
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    assert_eq!(MAX.wrapping_add(MAX), -2);
    assert_validation(&error, "amount overflow");
}

#[tokio::test]
async fn an_item_total_that_overflows_is_rejected() {
    let (_, service) = service().await;

    // subtotal (MAX) - discount (0) + tax (MAX) cannot be represented.
    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                tax_total: Some(MAX),
                total: None,
                ..item(MAX, 1)
            }],
            subtotal: MAX,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    assert_validation(&error, "item total overflow");
}

#[tokio::test]
async fn tax_lines_that_overflow_the_tax_total_are_rejected() {
    let (_, service) = service().await;

    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![item(1, 1)],
            tax_lines: vec![
                CreateOrderTaxLineInput {
                    item_index: Some(0),
                    shipping_option_id: None,
                    name: "Sales tax".to_owned(),
                    rate_bps: 1_000,
                    amount: MAX,
                    is_tax_inclusive: false,
                },
                CreateOrderTaxLineInput {
                    item_index: Some(0),
                    shipping_option_id: None,
                    name: "Duty".to_owned(),
                    rate_bps: 1_000,
                    amount: MAX,
                    is_tax_inclusive: false,
                },
            ],
            subtotal: 1,
            total: 1,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    assert_eq!(MAX.wrapping_add(MAX), -2);
    assert_validation(&error, "tax amount overflow");
}

#[tokio::test]
async fn an_order_total_that_overflows_is_rejected() {
    let (_, service) = service().await;

    // The lines total the whole range, so shipping can no longer be added.
    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![item(MAX, 1)],
            subtotal: MAX,
            shipping_total: 1,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    assert_validation(&error, "order total overflow");
}

#[tokio::test]
async fn a_refund_that_overflows_the_refunded_total_is_rejected() {
    let (_, service) = service().await;
    let order = order_with_line(&service, MAX, 1, 0).await;
    assert_eq!(order.total, MAX);

    // The largest refund the order can absorb leaves one unit unrefunded.
    service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: MAX - 1,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap();

    // Adding to the running total overflows before the "exceeds the
    // refundable total" comparison can run.
    let error = service
        .add_refund(
            &order.id,
            RefundInput {
                payment_id: PaymentId::new(),
                amount: 2,
                currency: "USD".to_owned(),
                reason: None,
            },
        )
        .await
        .unwrap_err();

    assert_eq!((MAX - 1).wrapping_add(2), i64::MIN);
    assert_validation(&error, "refund amount overflow");
}

#[tokio::test]
async fn an_added_edit_line_that_overflows_is_rejected() {
    let (_, service) = service().await;
    let order = order_with_line(&service, 100, 1, 0).await;

    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Cable".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(MAX),
                    before_quantity: None,
                    after_quantity: 2,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert_validation(&error, "edit amount overflow");
}

#[tokio::test]
async fn an_updated_edit_line_that_overflows_is_rejected() {
    let (_, service) = service().await;
    let order = order_with_line(&service, MAX, 1, 0).await;

    // Growing the line by two units multiplies the largest unit price by two.
    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: Some(order.items[0].id.clone()),
                    action: OrderEditAction::Update,
                    product_id: None,
                    variant_id: None,
                    product_title: None,
                    variant_title: None,
                    sku: None,
                    unit_price: None,
                    before_quantity: Some(1),
                    after_quantity: 3,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert_eq!(MAX.wrapping_mul(2), -2);
    assert_validation(&error, "edit amount overflow");
}

#[tokio::test]
async fn an_edit_that_overflows_the_order_subtotal_is_rejected() {
    let (_, service) = service().await;
    let order = order_with_line(&service, MAX, 1, 0).await;

    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Sample".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(1),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert_validation(&error, "edit subtotal overflow");
}

#[tokio::test]
async fn an_edit_that_overflows_the_order_total_is_rejected() {
    let (_, service) = service().await;
    // Shipping pushes the total past the subtotal, so one more unit overflows
    // the total while the subtotal itself still fits.
    let order = order_with_line(&service, MAX - 1, 1, 1).await;
    assert_eq!(order.subtotal, MAX - 1);
    assert_eq!(order.total, MAX);

    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Sample".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(1),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert_validation(&error, "edit total overflow");
}
