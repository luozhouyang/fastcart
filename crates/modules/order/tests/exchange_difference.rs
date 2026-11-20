//! Price-difference settlement for accepted exchanges and claims.
//!
//! A replacement is rarely priced like the line it stands in for. Accepting an
//! exchange or a claim without settling the difference silently moves that
//! money out of the merchant's books: a dearer replacement is handed over
//! without charging the difference, and a cheaper one is handed over without
//! crediting it.
//!
//! Confirming therefore settles the difference over the record's replacement
//! lines as
//!
//! ```text
//! difference = (replacement unit price - replaced unit price) * quantity
//! ```
//!
//! where the replaced unit price is the *current* `order_items.unit_price` (a
//! confirmed order edit rewrites the row an after-sales record points at) and
//! the replacement unit price is supplied by the caller, because the order
//! module does not price catalog variants. A positive total is owed by the
//! customer, a negative one is owed to them.
//!
//! The total is written once, as a signed order-level adjustment, so the order
//! keeps the identity `create_order` guarantees:
//! `orders.total == sum(order_items.total) + shipping_total + sum(order_adjustments)`.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, LineItemId, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    ClaimItemInput, ClaimItemType, CreateClaimInput, CreateExchangeInput, CreateOrderInput,
    CreateOrderItemInput, ExchangeItemInput, ExchangeStatus, Order, OrderExchange, OrderService,
    ReplacementUnitPrice,
};

async fn service() -> (StoreDatabase, OrderService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone());
    (db, service)
}

/// Places a one-line order with no discounts, taxes or shipping.
async fn order(service: &OrderService, quantity: i64, unit_price: i64) -> Order {
    order_with_shipping(service, quantity, unit_price, 0).await
}

/// Places a one-line order with a shipping fee, like a delivery order.
async fn order_with_shipping(
    service: &OrderService,
    quantity: i64,
    unit_price: i64,
    shipping_total: i64,
) -> Order {
    let subtotal = unit_price * quantity;
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                ..CreateOrderItemInput::default()
            }],
            subtotal,
            shipping_total,
            total: subtotal + shipping_total,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

/// Exchanges `quantity` units of the order's first line for `replacement`.
async fn exchange(
    service: &OrderService,
    order: &Order,
    replacement: VariantId,
    quantity: i64,
) -> OrderExchange {
    exchange_with(service, order, replacement, quantity, quantity).await
}

/// Exchanges `inbound_quantity` units for `outbound_quantity` replacement
/// units; the outbound side is not capped by the inbound one.
async fn exchange_with(
    service: &OrderService,
    order: &Order,
    replacement: VariantId,
    inbound_quantity: i64,
    outbound_quantity: i64,
) -> OrderExchange {
    service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: inbound_quantity,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: outbound_quantity,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap()
}

/// The caller-resolved unit price of one replacement line.
fn price(order_item_id: &LineItemId, unit_price: i64) -> ReplacementUnitPrice {
    ReplacementUnitPrice {
        order_item_id: order_item_id.clone(),
        unit_price,
    }
}

/// The identity `create_order` guarantees and every later flow must preserve.
fn assert_order_identity(order: &Order) {
    let line_totals: i64 = order.items.iter().map(|item| item.total).sum();
    let adjustments: i64 = order.adjustments.iter().map(|entry| entry.amount).sum();
    assert_eq!(
        line_totals + order.shipping_total + adjustments,
        order.total,
        "orders.total must be the sum of line totals plus shipping plus adjustments"
    );
}

fn assert_validation(error: &AppError, needle: &str) {
    assert!(
        matches!(error, AppError::Validation(message) if message.contains(needle)),
        "expected a validation error mentioning {needle:?}, got {error:?}"
    );
}

#[tokio::test]
async fn a_dearer_replacement_is_charged_to_the_order() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_500)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(500));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_500);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_eq!(order.adjustments[0].code, "exchange_difference");
    assert_eq!(order.adjustments[0].amount, 500);
    assert_order_identity(&order);
}

#[tokio::test]
async fn a_cheaper_replacement_is_owed_back_to_the_customer() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 400)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(-600));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 1_400);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_eq!(order.adjustments[0].amount, -600);
    assert_order_identity(&order);
}

#[tokio::test]
async fn an_equal_priced_replacement_writes_no_adjustment() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_000)])
        .await
        .unwrap();

    // The difference is settled (and so reported), but a zero difference is
    // not booked as an adjustment.
    assert_eq!(accepted.difference_total, Some(0));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_000);
    assert!(order.adjustments.is_empty(), "{:?}", order.adjustments);
    assert_order_identity(&order);
}

#[tokio::test]
async fn the_settlement_keeps_the_order_identity_with_shipping() {
    let (_db, service) = service().await;
    let order = order_with_shipping(&service, 2, 1_000, 250).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_500)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(500));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_750);
    // The adjustment is added on top of the lines and the shipping, so
    // `orders.total` still describes the same purchase.
    assert_order_identity(&order);
}

#[tokio::test]
async fn the_difference_scales_with_the_replacement_quantity() {
    let (_db, service) = service().await;
    let order = order(&service, 3, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 2).await;

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_200)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(400));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 3_400);
    assert_order_identity(&order);
}

#[tokio::test]
async fn the_difference_uses_the_current_line_price() {
    let (db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    // A confirmed order edit rewrites the lines an after-sales record points
    // at. The row is rewritten directly here to model that: what matters is
    // that the difference reads the current row, not a snapshot taken when the
    // exchange was created.
    sqlx::query(
        "UPDATE order_items SET unit_price = 1500, subtotal = 3000, total = 3000 WHERE id = ?",
    )
    .bind(order.items[0].id.as_str())
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("UPDATE orders SET subtotal = 3000, total = 3000 WHERE id = ?")
        .bind(order.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_000)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(-500));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_500);
    assert_order_identity(&order);
}

#[tokio::test]
async fn repeated_acceptance_settles_the_difference_once() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let first = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_500)])
        .await
        .unwrap();
    assert_eq!(first.difference_total, Some(500));

    // A replay must not book the difference again, whatever prices it carries.
    let replay = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 9_000)])
        .await
        .unwrap();

    assert_eq!(replay.difference_total, Some(500));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_500);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_order_identity(&order);
}

#[tokio::test]
async fn a_replacement_without_a_price_settles_nothing() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    // The caller could not resolve a price for the replacement variant, so
    // there is no known difference to settle for that line.
    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(0));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_000);
    assert!(order.adjustments.is_empty(), "{:?}", order.adjustments);
    assert_order_identity(&order);
}

#[tokio::test]
async fn prices_for_lines_the_record_does_not_replace_are_rejected() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let exchange = exchange(&service, &order, VariantId::new(), 1).await;

    let error = service
        .accept_exchange(&order.id, &exchange.id, &[price(&LineItemId::new(), 1_500)])
        .await
        .unwrap_err();

    assert_validation(&error, "does not replace");
    // The failed call left nothing behind: no settlement and no half-accepted
    // record.
    let exchange = service.get_exchange(&order.id, &exchange.id).await.unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Requested);
    assert_eq!(exchange.difference_total, None);
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_000);
    assert!(order.adjustments.is_empty(), "{:?}", order.adjustments);
}

#[tokio::test]
async fn a_difference_that_would_leave_a_negative_total_is_rejected() {
    let (_db, service) = service().await;
    let order = order(&service, 1, 1_000).await;
    // The outbound line is not capped by the inbound quantity, so the credit
    // can exceed everything the customer was charged.
    let exchange = exchange_with(&service, &order, VariantId::new(), 1, 2).await;

    let error = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 100)])
        .await
        .unwrap_err();

    assert!(
        matches!(error, AppError::Conflict(_)),
        "expected a conflict for a negative total, got {error:?}"
    );
    let exchange = service.get_exchange(&order.id, &exchange.id).await.unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Requested);
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 1_000);
    assert_order_identity(&order);
}

#[tokio::test]
async fn claim_replacements_settle_the_difference_too() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: Metadata::empty(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    item_type: ClaimItemType::Replace,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let accepted = service
        .accept_claim(&order.id, &claim.id, &[price(&order.items[0].id, 1_300)])
        .await
        .unwrap();

    assert_eq!(accepted.difference_total, Some(600));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_600);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_eq!(order.adjustments[0].code, "claim_difference");
    assert_eq!(order.adjustments[0].amount, 600);
    assert_order_identity(&order);

    let replay = service
        .accept_claim(&order.id, &claim.id, &[price(&order.items[0].id, 9_000)])
        .await
        .unwrap();
    assert_eq!(replay.difference_total, Some(600));
    assert_eq!(service.get(&order.id).await.unwrap().total, 2_600);
}

#[tokio::test]
async fn refund_claim_lines_never_carry_a_difference() {
    let (_db, service) = service().await;
    let order = order(&service, 2, 1_000).await;
    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
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

    // Only replacement lines are priced: a price for a refunded line is a
    // caller mistake, not a silent surcharge.
    let error = service
        .accept_claim(&order.id, &claim.id, &[price(&order.items[0].id, 1_500)])
        .await
        .unwrap_err();

    assert_validation(&error, "does not replace");

    let accepted = service
        .accept_claim(&order.id, &claim.id, &[])
        .await
        .unwrap();
    assert_eq!(accepted.difference_total, Some(0));
    let order = service.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_000);
    assert!(order.adjustments.is_empty(), "{:?}", order.adjustments);
    assert_order_identity(&order);
}
