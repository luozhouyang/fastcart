//! Line-item consistency for confirmed order edits.
//!
//! `create_order` guarantees that the persisted order totals are exactly the
//! sums of the materialized lines (see `resolve_order`): `subtotal` is the sum
//! of line subtotals, `discount_total` the sum of line discounts, and `total`
//! is the sum of line totals plus shipping plus order adjustments.
//!
//! `confirm_order_edit` has to keep that identity: a confirmed edit lands on
//! `order_items` — additions insert a line, quantity changes rewrite the line's
//! quantity and amounts, removals keep the row with a zero quantity — and the
//! order totals are read back from the lines. Every consumer that reads order
//! lines (fulfillment, returns, claims, exchanges, invoices, reports) then sees
//! the same purchase the totals describe.
//!
//! Constraints the implementation has to respect:
//!
//! * `order_return_items`, `order_claim_items` and `order_exchange_items`
//!   reference `order_items.id` with `ON DELETE RESTRICT` (`order_fulfillment_items`
//!   and `order_tax_lines` cascade), so line rows are never deleted. A removal is
//!   expressed as a zero quantity on the existing row.
//! * After-sales records that already reference a line keep working: their
//!   `order_item_id` must still resolve to a live `order_items` row.
//! * Tax is levied per line: an item-scoped tax line shrinks with its line and
//!   shipping-scoped tax is preserved with the shipping, so the breakdown still
//!   adds up to `orders.tax_total`. The service tests cover that rule in
//!   `order_tax_lines_follow_their_base_and_keep_adding_up`.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, StoreId};
use fastcart_module_order::{
    CreateOrderEditInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput,
    CreateReturnInput, FulfillmentInput, FulfillmentItemInput, FulfillmentStatus, Order,
    OrderEditAction, OrderEditItemInput, OrderService, ReceiveReturnInput, ReturnItemInput,
};
use serde_json::json;

async fn service() -> (StoreDatabase, OrderService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone());
    (db, service)
}

/// Places an order with exactly one shippable line.
///
/// The line carries the whole order allocation so the order-level totals are
/// also the line totals (`tax_total` is fully item-scoped, `discount_total` is
/// fully line-scoped).
async fn single_line_order(
    service: &OrderService,
    quantity: i64,
    unit_price: i64,
    discount_total: i64,
    tax_total: i64,
) -> Order {
    let subtotal = unit_price * quantity;
    let total = subtotal - discount_total + tax_total;
    let tax_lines = if tax_total > 0 {
        vec![CreateOrderTaxLineInput {
            item_index: Some(0),
            shipping_option_id: None,
            name: "Sales tax".to_owned(),
            rate_bps: 1_000,
            amount: tax_total,
            is_tax_inclusive: false,
        }]
    } else {
        Vec::new()
    };
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                subtotal: Some(subtotal),
                discount_total: Some(discount_total),
                tax_total: Some(tax_total),
                total: Some(total),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal,
            discount_total,
            tax_total,
            tax_lines,
            total,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

/// `orders` amounts must describe exactly what `order_items` holds.
fn assert_totals_are_derived_from_lines(order: &Order) {
    let items_subtotal: i64 = order.items.iter().map(|item| item.subtotal).sum();
    let items_discount: i64 = order.items.iter().map(|item| item.discount_total).sum();
    let items_tax: i64 = order.items.iter().map(|item| item.tax_total).sum();
    let items_total: i64 = order.items.iter().map(|item| item.total).sum();
    let adjustments: i64 = order.adjustments.iter().map(|value| value.amount).sum();

    assert_eq!(
        items_subtotal, order.subtotal,
        "orders.subtotal must be the sum of the materialized line subtotals"
    );
    assert_eq!(
        items_discount, order.discount_total,
        "orders.discount_total must be the sum of the materialized line discounts"
    );
    assert_eq!(
        items_tax, order.tax_total,
        "orders.tax_total must be the sum of the materialized line taxes"
    );
    assert_eq!(
        items_total + order.shipping_total + adjustments,
        order.total,
        "orders.total must be the sum of line totals plus shipping plus adjustments"
    );
}

fn add_item(title: &str, sku: &str, unit_price: i64, quantity: i64) -> OrderEditItemInput {
    OrderEditItemInput {
        order_item_id: None,
        action: OrderEditAction::Add,
        product_id: None,
        variant_id: None,
        product_title: Some(title.to_owned()),
        variant_title: None,
        sku: Some(sku.to_owned()),
        unit_price: Some(unit_price),
        before_quantity: None,
        after_quantity: quantity,
        requires_shipping: None,
        is_tax_inclusive: None,
    }
}

fn update_item(order: &Order, index: usize, after_quantity: i64) -> OrderEditItemInput {
    OrderEditItemInput {
        order_item_id: Some(order.items[index].id.clone()),
        action: OrderEditAction::Update,
        product_id: None,
        variant_id: None,
        product_title: None,
        variant_title: None,
        sku: None,
        unit_price: None,
        before_quantity: Some(order.items[index].quantity),
        after_quantity,
        requires_shipping: None,
        is_tax_inclusive: None,
    }
}

fn remove_item(order: &Order, index: usize) -> OrderEditItemInput {
    OrderEditItemInput {
        order_item_id: Some(order.items[index].id.clone()),
        action: OrderEditAction::Remove,
        product_id: None,
        variant_id: None,
        product_title: None,
        variant_title: None,
        sku: None,
        unit_price: None,
        before_quantity: Some(order.items[index].quantity),
        after_quantity: 0,
        requires_shipping: None,
        is_tax_inclusive: None,
    }
}

fn edit_input(items: Vec<OrderEditItemInput>) -> CreateOrderEditInput {
    CreateOrderEditInput {
        reason: None,
        metadata: Metadata::default(),
        items,
    }
}

#[tokio::test]
async fn an_added_line_is_materialized_in_order_items() {
    let (db, service) = service().await;
    let order = single_line_order(&service, 2, 1_000, 0, 0).await;

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("add gift".to_owned()),
                metadata: Metadata::default(),
                items: vec![add_item("Gift card", "GIFT-1", 500, 1)],
            },
        )
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(
        reloaded.items.len(),
        2,
        "the added line must exist as an order item, not only as an edit line"
    );
    let added = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be readable from the order");
    assert_eq!(added.quantity, 1);
    assert_eq!(added.unit_price, 500);
    assert_eq!(added.subtotal, 500);
    assert_eq!(added.discount_total, 0);
    assert_eq!(added.tax_total, 0);
    assert_eq!(added.total, 500);
    let original = reloaded
        .items
        .iter()
        .find(|item| item.sku.is_none())
        .expect("the purchased line must still exist");
    assert_eq!(original.quantity, 2);
    assert_eq!(original.subtotal, 2_000);

    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.total, 2_500);
    assert_totals_are_derived_from_lines(&reloaded);

    // The aggregate must not be the only place the line exists.
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_items WHERE order_id = ?")
        .bind(order.id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 2);

    // Confirming again is idempotent and must not duplicate the line.
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let again = service.get(&order.id).await.unwrap();
    assert_eq!(again.items.len(), 2);
    assert_totals_are_derived_from_lines(&again);
}

#[tokio::test]
async fn a_quantity_change_lands_on_the_line_and_keeps_the_amounts_derived() {
    let (_db, service) = service().await;
    let order = single_line_order(&service, 4, 1_000, 400, 0).await;

    let edit = service
        .create_order_edit(&order.id, edit_input(vec![update_item(&order, 0, 2)]))
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    // The order-level amounts are unchanged from today's behaviour.
    assert_eq!(reloaded.subtotal, 2_000);
    assert_eq!(reloaded.discount_total, 200);
    assert_eq!(reloaded.total, 1_800);
    // ...but the line now describes the same purchase.
    assert_eq!(reloaded.items.len(), 1);
    assert_eq!(reloaded.items[0].unit_price, 1_000);
    assert_eq!(reloaded.items[0].quantity, 2);
    assert_eq!(reloaded.items[0].subtotal, 2_000);
    assert_eq!(reloaded.items[0].discount_total, 200);
    assert_eq!(reloaded.items[0].total, 1_800);
    assert_totals_are_derived_from_lines(&reloaded);
}

#[tokio::test]
async fn a_removed_line_keeps_its_row_with_a_zero_quantity() {
    let (db, service) = service().await;
    let order = single_line_order(&service, 2, 1_000, 0, 0).await;
    let line_id = order.items[0].id.clone();

    let edit = service
        .create_order_edit(&order.id, edit_input(vec![remove_item(&order, 0)]))
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(
        reloaded.items.len(),
        1,
        "the row must survive: after-sales records reference it with ON DELETE RESTRICT"
    );
    assert_eq!(reloaded.items[0].quantity, 0);
    assert_eq!(reloaded.items[0].subtotal, 0);
    assert_eq!(reloaded.items[0].total, 0);
    assert_eq!(reloaded.subtotal, 0);
    assert_eq!(reloaded.total, 0);
    assert_totals_are_derived_from_lines(&reloaded);

    // The historical row survives a reload from SQLite, under its original id.
    let persisted: String = sqlx::query_scalar("SELECT id FROM order_items WHERE order_id = ?")
        .bind(order.id.as_str())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(persisted, line_id.as_str());

    // A removed unit is no longer returnable: the order does not contain it.
    let error = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: None,
                reason_id: None,
                refund_id: None,
                metadata: Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: line_id,
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn existing_return_references_survive_a_quantity_reduction() {
    let (_db, service) = service().await;
    let order = single_line_order(&service, 3, 1_000, 0, 0).await;
    let line_id = order.items[0].id.clone();
    let returned = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: Some("damaged".to_owned()),
                reason_id: None,
                refund_id: None,
                metadata: Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: line_id.clone(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    service
        .request_return(&order.id, &returned.id)
        .await
        .unwrap();

    // Shrink the line from 3 to 2: still above the one unit already returned.
    let edit = service
        .create_order_edit(&order.id, edit_input(vec![update_item(&order, 0, 2)]))
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items[0].quantity, 2);
    assert_eq!(reloaded.subtotal, 2_000);
    assert_totals_are_derived_from_lines(&reloaded);

    // The return still resolves to a live order item and can be received.
    let listed = service.list_returns(&order.id).await.unwrap();
    assert_eq!(listed[0].items[0].order_item_id, line_id);
    let received = service
        .receive_return(&order.id, &returned.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(received.items[0].received_quantity, 1);
}

#[tokio::test]
async fn an_edit_cannot_shrink_a_line_below_committed_after_sales_quantities() {
    let (_db, service) = service().await;
    let order = single_line_order(&service, 3, 1_000, 0, 0).await;
    let returned = service
        .create_return(
            &order.id,
            CreateReturnInput {
                reason: None,
                reason_id: None,
                refund_id: None,
                metadata: Metadata::default(),
                items: vec![ReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    service
        .request_return(&order.id, &returned.id)
        .await
        .unwrap();

    // Two units are already committed to the return: a line of one unit would
    // make the return describe more units than the order holds.
    let error = service
        .create_order_edit(&order.id, edit_input(vec![update_item(&order, 0, 1)]))
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(_)),
        "expected a conflict, got {error:?}"
    );
    let error = service
        .create_order_edit(&order.id, edit_input(vec![remove_item(&order, 0)]))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    // Shrinking to exactly the committed quantity is allowed.
    let edit = service
        .create_order_edit(&order.id, edit_input(vec![update_item(&order, 0, 2)]))
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items[0].quantity, 2);
    assert_totals_are_derived_from_lines(&reloaded);
}

#[tokio::test]
async fn fulfillment_references_survive_a_quantity_reduction() {
    let (_db, service) = service().await;
    let order = single_line_order(&service, 4, 1_000, 0, 0).await;
    service.confirm(&order.id).await.unwrap();
    let line_id = order.items[0].id.clone();

    let fulfilled = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_data: json!({}),
                items: vec![FulfillmentItemInput {
                    order_item_id: line_id.clone(),
                    quantity: 2,
                }],
                partial: true,
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(fulfilled.fulfillments[0].items[0].quantity, 2);

    // Reduce the line from 4 to 3; the shipped units stay shipped.
    let edit = service
        .create_order_edit(&order.id, edit_input(vec![update_item(&order, 0, 3)]))
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.items[0].quantity, 3);
    assert_eq!(
        reloaded.fulfillments[0].items[0].order_item_id, line_id,
        "the fulfillment must keep pointing at the same order item"
    );
    assert_totals_are_derived_from_lines(&reloaded);

    // Exactly one unit remains shippable.
    let completed = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_data: json!({}),
                items: vec![FulfillmentItemInput {
                    order_item_id: line_id.clone(),
                    quantity: 1,
                }],
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.fulfillment_status, FulfillmentStatus::Fulfilled);
    let error = service
        .fulfill(&order.id, FulfillmentInput::default())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn an_added_line_is_shippable() {
    let (_db, service) = service().await;
    let order = single_line_order(&service, 1, 1_000, 0, 0).await;
    service.confirm(&order.id).await.unwrap();

    let edit = service
        .create_order_edit(
            &order.id,
            edit_input(vec![add_item("Gift card", "GIFT-1", 500, 1)]),
        )
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();

    let reloaded = service.get(&order.id).await.unwrap();
    let added = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be readable from the order");
    let fulfilled = service
        .fulfill(
            &order.id,
            FulfillmentInput {
                provider_data: json!({}),
                items: vec![FulfillmentItemInput {
                    order_item_id: added.id.clone(),
                    quantity: 1,
                }],
                ..FulfillmentInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(fulfilled.fulfillments.len(), 1);
    assert_eq!(fulfilled.fulfillments[0].items[0].order_item_id, added.id);
}
