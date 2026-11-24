//! Confirming an order edit settles both sides of the change.
//!
//! The line rewrite is atomic with the order totals (ADR 0009), and now with
//! the stock movements too: an edit that cannot take its units leaves the order
//! untouched. The money side is deliberately a separate, idempotent call, the
//! same shape claims and exchanges use, and `payment_summary` is the read side
//! that says what is still owed or owed back.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CheckoutId, Metadata, StoreId, VariantId};
use fastcart_module_inventory::{
    CreateInventoryItemInput, CreateLocationInput, InventoryService, ReservationLine,
    ReservationReference,
};
use fastcart_module_order::{
    CreateOrderEditInput, CreateOrderInput, CreateOrderItemInput, FulfillmentInput,
    FulfillmentItemInput, FulfillmentStatus, Order, OrderEditAction, OrderEditItemInput,
    OrderService, OrderStatus,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, PaymentService, ProviderRegistry,
};
use fastcart_workflows::AfterSalesWorkflow;

const STOCK: i64 = 20;

struct Fixture {
    workflow: AfterSalesWorkflow,
    orders: OrderService,
    inventory: InventoryService,
    payments: PaymentService,
    variant_id: VariantId,
    item_id: fastcart_kernel::InventoryItemId,
    location_id: fastcart_kernel::InventoryLocationId,
}

async fn fixture() -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let inventory = InventoryService::new(db.clone());
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let variant_id = VariantId::new();
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant_id.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    inventory
        .adjust_stock(&item.id, &location.id, STOCK, "restock", None)
        .await
        .unwrap();
    Fixture {
        workflow: AfterSalesWorkflow::new(
            db.clone(),
            PaymentService::with_registry(db.clone(), providers.clone()),
        ),
        orders: OrderService::new(db.clone()),
        inventory,
        payments: PaymentService::with_registry(db, providers),
        variant_id,
        item_id: item.id,
        location_id: location.id,
    }
}

/// Places a one-line order for `quantity` units of the fixture variant.
///
/// The order is created directly, without a checkout, so it holds no committed
/// reservation until [`commit_stock`] runs. That keeps the two inventory
/// sources — a real checkout and a manual order — testable.
async fn order(fixture: &Fixture, quantity: i64, unit_price: i64) -> Order {
    let order = fixture
        .orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                variant_id: Some(fixture.variant_id.clone()),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    // Confirmed orders are the ones an admin can fulfil and edit; leave the
    // pending state to the order module's own tests.
    fixture.orders.confirm(&order.id).await.unwrap()
}

/// Takes the order's units out of stock the way checkout does.
async fn commit_stock(fixture: &Fixture, order: &Order, quantity: i64) {
    let checkout = CheckoutId::new();
    let reservations = fixture
        .inventory
        .reserve(
            vec![ReservationLine::new(fixture.variant_id.clone(), quantity)],
            ReservationReference::new(checkout.clone()),
            3_600,
        )
        .await
        .unwrap();
    assert_eq!(reservations.len(), 1);
    fixture
        .inventory
        .commit(&checkout, &order.id)
        .await
        .unwrap();
}

async fn level(fixture: &Fixture) -> i64 {
    fixture
        .inventory
        .get_level(&fixture.item_id, &fixture.location_id)
        .await
        .map(|level| level.stocked_quantity)
        .unwrap_or(0)
}

fn edit(
    action: OrderEditAction,
    order_item_id: Option<fastcart_kernel::LineItemId>,
    before: Option<i64>,
    after: i64,
) -> OrderEditItemInput {
    OrderEditItemInput {
        order_item_id,
        action,
        product_id: None,
        variant_id: None,
        product_title: if action == OrderEditAction::Add {
            Some("Extra".to_owned())
        } else {
            None
        },
        variant_title: None,
        sku: None,
        unit_price: if action == OrderEditAction::Add {
            Some(500)
        } else {
            None
        },
        before_quantity: before,
        after_quantity: after,
        requires_shipping: Some(true),
    }
}

fn edited(order: &Order, index: usize, after_quantity: i64) -> CreateOrderEditInput {
    CreateOrderEditInput {
        reason: None,
        metadata: Metadata::default(),
        items: vec![edit(
            OrderEditAction::Update,
            Some(order.items[index].id.clone()),
            Some(order.items[index].quantity),
            after_quantity,
        )],
    }
}

/// Captures the whole order through the sandbox provider, as checkout would.
async fn pay(fixture: &Fixture, order: &Order) {
    let session = fixture
        .payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: order.currency_code.clone(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    fixture.payments.capture(session.id.as_str()).await.unwrap();
}

#[tokio::test]
async fn reducing_an_unshipped_line_returns_its_units() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;
    let stock_after_sale = STOCK - 4;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 3))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    assert_eq!(level(&fixture).await, stock_after_sale + 1);
    let movements = fixture
        .inventory
        .list_movements(&fixture.item_id)
        .await
        .unwrap();
    assert!(
        movements
            .iter()
            .any(|movement| { movement.reason == "order_edit" && movement.quantity_delta == 1 })
    );
}

#[tokio::test]
async fn increasing_a_line_or_adding_one_consumes_stock() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    let grow = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 5))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &grow.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, STOCK - 5);

    let add = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: Some(fixture.variant_id.clone()),
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 2,
                    requires_shipping: Some(true),
                }],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, STOCK - 7);
}

#[tokio::test]
async fn a_manual_order_reduction_does_not_invent_stock() {
    let fixture = fixture().await;
    // No checkout ran, so the order never took the four units out of stock.
    let order = order(&fixture, 4, 1_000).await;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    assert_eq!(level(&fixture).await, STOCK);
}

#[tokio::test]
async fn an_edit_cannot_take_back_shipped_units() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;
    fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                }],
                partial: true,
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();

    // Preparing the edit already fails: two units are gone.
    let error = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    // A draft prepared before the shipment is re-checked at confirmation.
    let draft = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
                partial: true,
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &draft.id)
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(
        reloaded.items[0].quantity, 4,
        "the rejected edit changed nothing"
    );
}

#[tokio::test]
async fn insufficient_stock_rolls_the_edit_back() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;

    // Ask for ten more units when the shelf holds none.
    fixture
        .inventory
        .adjust_stock(
            &fixture.item_id,
            &fixture.location_id,
            -STOCK,
            "shrink",
            None,
        )
        .await
        .unwrap();

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: Some(fixture.variant_id.clone()),
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(1_000),
                    before_quantity: None,
                    after_quantity: 10,
                    requires_shipping: Some(true),
                }],
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items.len(), 1, "the added line must not survive");
    assert_eq!(reloaded.total, 1_000);
    let stored = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(
        stored.status,
        fastcart_module_order::OrderEditStatus::Draft,
        "a failed confirmation must leave the edit draft"
    );
}

#[tokio::test]
async fn replaying_a_confirmation_moves_stock_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    let edit = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 2))
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let once = level(&fixture).await;
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(level(&fixture).await, once, "replay must not restock twice");
}

#[tokio::test]
async fn a_dearer_edit_becomes_payable_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                }],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let confirmed = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.difference_total, Some(500));
    assert_eq!(
        level(&fixture).await,
        STOCK,
        "a line that does not ship must not move stock"
    );

    let first = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap();
    assert_eq!(first.session.amount, 500);
    let second = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap();
    assert_eq!(
        first.session.id, second.session.id,
        "the difference must not open a second payable"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 1_500);
    assert_eq!(summary.captured_total, 1_000);
    assert_eq!(summary.outstanding, 500);
    assert_eq!(summary.refund_due(), 0);
}

#[tokio::test]
async fn a_cheaper_edit_refunds_what_was_captured_once() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

    // Remove the only line: the order now totals nothing and the whole
    // captured amount is owed back.
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("customer changed their mind".to_owned()),
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let confirmed = fixture
        .orders
        .get_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.difference_total, Some(-1_000));

    let first = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert!(first.order_refund.is_some());
    assert_eq!(first.order_refund.as_ref().unwrap().amount, 1_000);

    let second = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(
        first.order_refund.as_ref().unwrap().id,
        second.order_refund.as_ref().unwrap().id,
        "the refund must be recorded once"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 0);
    assert_eq!(summary.captured_total, 1_000);
    assert_eq!(summary.refunded_total, 1_000);
    assert_eq!(summary.outstanding, 0);
}

#[tokio::test]
async fn a_cheaper_edit_refunds_past_a_lowered_but_nonzero_total() {
    let fixture = fixture().await;
    // Two digital lines, so the test is about money only.
    let order = fixture
        .orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![
                CreateOrderItemInput {
                    product_title: "Expensive".to_owned(),
                    unit_price: 1_000,
                    quantity: 1,
                    requires_shipping: false,
                    ..CreateOrderItemInput::default()
                },
                CreateOrderItemInput {
                    product_title: "Cheap".to_owned(),
                    unit_price: 500,
                    quantity: 1,
                    requires_shipping: false,
                    ..CreateOrderItemInput::default()
                },
            ],
            subtotal: 1_500,
            total: 1_500,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let order = fixture.orders.confirm(&order.id).await.unwrap();
    pay(&fixture, &order).await;

    // Removing the expensive line leaves a total (500) below the refund the
    // edit owes (1000). The refund must be measured against what was captured,
    // not against the lowered total.
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("dropped the expensive line".to_owned()),
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let refund = fixture
        .workflow
        .refund_order_edit_difference(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(refund.order_refund.as_ref().unwrap().amount, 1_000);

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.total, 500);
    assert_eq!(summary.captured_total, 1_500);
    assert_eq!(summary.refunded_total, 1_000);
    assert_eq!(summary.outstanding, 0);
}

#[tokio::test]
async fn growing_a_fulfilled_order_reopens_fulfillment() {
    let fixture = fixture().await;
    let order = order(&fixture, 2, 1_000).await;
    commit_stock(&fixture, &order, 2).await;
    let fulfilled = fixture
        .orders
        .fulfill(&order.id, FulfillmentInput::default())
        .await
        .unwrap();
    assert_eq!(fulfilled.status, OrderStatus::Fulfilled);
    assert_eq!(fulfilled.fulfillment_status, FulfillmentStatus::Fulfilled);

    // Add a unit: the order now holds one unit nobody shipped yet, so leaving
    // it `fulfilled` would make `fulfill` refuse to ship that unit.
    let add = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Extra".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(true),
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(confirmed.order.status, OrderStatus::Processing);
    assert_eq!(
        confirmed.order.fulfillment_status,
        FulfillmentStatus::PartiallyFulfilled
    );

    let added_line = confirmed
        .order
        .items
        .iter()
        .find(|item| item.product_title == "Extra")
        .expect("the edit materialized the added line");
    let shipped = fixture
        .orders
        .fulfill(
            &order.id,
            FulfillmentInput {
                items: vec![FulfillmentItemInput {
                    order_item_id: added_line.id.clone(),
                    quantity: 1,
                }],
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(shipped.status, OrderStatus::Fulfilled);
    assert_eq!(shipped.fulfillment_status, FulfillmentStatus::Fulfilled);
}

#[tokio::test]
async fn stock_demand_is_summed_across_edit_lines() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    // Leave five units: each line fits on its own, but together they do not.
    fixture
        .inventory
        .adjust_stock(&fixture.item_id, &fixture.location_id, -15, "shrink", None)
        .await
        .unwrap();

    let add_line = |quantity: i64| OrderEditItemInput {
        order_item_id: None,
        action: OrderEditAction::Add,
        product_id: None,
        variant_id: Some(fixture.variant_id.clone()),
        product_title: Some("Extra".to_owned()),
        variant_title: None,
        sku: None,
        unit_price: Some(100),
        before_quantity: None,
        after_quantity: quantity,
        requires_shipping: Some(true),
    };
    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![add_line(3), add_line(3)],
            },
        )
        .await
        .unwrap();
    let error = fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("insufficient stock"), "{message}");

    let reloaded = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items.len(), 1, "no part of the batch may apply");
}

#[tokio::test]
async fn a_stale_draft_settles_only_its_own_difference() {
    let fixture = fixture().await;
    let order = order(&fixture, 4, 1_000).await;
    commit_stock(&fixture, &order, 4).await;

    // Two drafts are prepared against the same 4000 total. Confirming the
    // addition first changes the order; the reduction's difference must then
    // measure against 4500 and settle only the 1000 it removed, not include
    // the addition a second time.
    let reduce = fixture
        .orders
        .create_order_edit(&order.id, edited(&order, 0, 3))
        .await
        .unwrap();
    let add = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: None,
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: Some(false),
                }],
            },
        )
        .await
        .unwrap();

    let added = fixture
        .workflow
        .confirm_order_edit(&order.id, &add.id)
        .await
        .unwrap();
    assert_eq!(added.order_edit.difference_total, Some(500));

    let reduced = fixture
        .workflow
        .confirm_order_edit(&order.id, &reduce.id)
        .await
        .unwrap();
    assert_eq!(reduced.order_edit.difference_total, Some(-1_000));
    assert_eq!(reduced.order.total, 3_500);
}

#[tokio::test]
async fn collecting_a_cheaper_edit_is_refused() {
    let fixture = fixture().await;
    let order = order(&fixture, 1, 1_000).await;
    pay(&fixture, &order).await;

    let edit = fixture
        .orders
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: Metadata::default(),
                items: vec![edit(
                    OrderEditAction::Remove,
                    Some(order.items[0].id.clone()),
                    Some(1),
                    0,
                )],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let error = fixture
        .workflow
        .collect_order_edit_difference(&order.id, &edit.id, "manual")
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );

    let summary = fixture.workflow.payment_summary(&order.id).await.unwrap();
    assert_eq!(summary.outstanding, -1_000);
    assert_eq!(summary.refund_due(), 1_000);
}
