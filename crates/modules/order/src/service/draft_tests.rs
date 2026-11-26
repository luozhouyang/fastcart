use fastcart_infrastructure::{StoreDatabase, fetch_pending_outbox};
use fastcart_kernel::{Pagination, StoreId, VariantId};
use fastcart_module_inventory::{CreateInventoryItemInput, CreateLocationInput, InventoryService};
use serde_json::json;

use super::OrderService;
use crate::after_sales::{
    CreateOrderEditInput, CreateReturnInput, OrderEditAction, OrderEditItemInput, ReturnItemInput,
};
use crate::domain::{
    CreateOrderInput, CreateOrderItemInput, DraftConversionSnapshot, DraftOrderTaxLineInput,
    OrderFilter, OrderStatus,
};

async fn service() -> (OrderService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    (OrderService::new(db.clone()), db)
}

fn draft_input(variant_id: Option<VariantId>, quantity: i64) -> CreateOrderInput {
    let total = 1_000 * quantity;
    CreateOrderInput {
        email: "buyer@example.com".to_owned(),
        currency_code: "USD".to_owned(),
        items: vec![CreateOrderItemInput {
            product_title: "Widget".to_owned(),
            variant_id,
            unit_price: 1_000,
            quantity,
            requires_shipping: true,
            ..CreateOrderItemInput::default()
        }],
        subtotal: total,
        total,
        shipping_address: json!({"country_code": "DE", "province_code": "BE"}),
        billing_address: json!({"country_code": "DE"}),
        ..CreateOrderInput::default()
    }
}

fn quantity_edit(order_item_id: &str, after_quantity: i64) -> CreateOrderEditInput {
    CreateOrderEditInput {
        reason: None,
        metadata: Default::default(),
        items: vec![OrderEditItemInput {
            order_item_id: Some(fastcart_kernel::LineItemId::from_string(
                order_item_id.to_owned(),
            )),
            action: OrderEditAction::Update,
            product_id: None,
            variant_id: None,
            product_title: None,
            variant_title: None,
            sku: None,
            unit_price: None,
            before_quantity: Some(2),
            after_quantity,
            requires_shipping: None,
            is_tax_inclusive: None,
        }],
    }
}

#[tokio::test]
async fn a_draft_order_starts_outside_the_lifecycle() {
    let (orders, db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 2))
        .await
        .unwrap();

    assert_eq!(draft.status, OrderStatus::Draft);
    assert!(draft.is_draft_order());
    assert_eq!(draft.status_history.len(), 1);
    assert_eq!(draft.status_history[0].status, OrderStatus::Draft);
    assert_eq!(
        draft.status_history[0].note.as_deref(),
        Some("draft order created")
    );

    let events = fetch_pending_outbox(db.pool(), 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "order.draft_created");

    let excluded = orders
        .list(
            OrderFilter {
                exclude_drafts: true,
                ..OrderFilter::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(excluded.count, 0);
    let included = orders
        .list(OrderFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(included.count, 1);
}

#[tokio::test]
async fn converting_a_draft_places_it_once() {
    let (orders, db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 1))
        .await
        .unwrap();

    let converted = orders.convert_draft_order(&draft.id).await.unwrap();
    assert_eq!(converted.status, OrderStatus::Pending);
    assert_eq!(
        converted.status_history.last().unwrap().note.as_deref(),
        Some("draft order converted")
    );

    let events = fetch_pending_outbox(db.pool(), 10).await.unwrap();
    let placed = events
        .iter()
        .find(|event| event.event_type == "order.placed")
        .expect("conversion publishes the placed event");
    let payload: serde_json::Value = serde_json::from_str(&placed.payload).unwrap();
    assert_eq!(payload["order_id"], converted.id.as_str());
    assert_eq!(payload["converted_from_draft"], true);

    let error = orders.convert_draft_order(&draft.id).await.unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_draft_edit_does_not_move_stock() {
    let (orders, db) = service().await;
    let inventory = InventoryService::new(db.clone());
    let variant = VariantId::new();
    let location = inventory
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap()
        .id;
    let item = inventory
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap()
        .id;
    inventory
        .adjust_stock(&item, &location, 10, "restock", None)
        .await
        .unwrap();

    let draft = orders
        .create_draft_order(draft_input(Some(variant.clone()), 2))
        .await
        .unwrap();
    let edit = orders
        .create_order_edit(&draft.id, quantity_edit(draft.items[0].id.as_str(), 5))
        .await
        .unwrap();
    let confirmed = orders
        .confirm_order_edit(&draft.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(
        confirmed.difference_total,
        Some(0),
        "a draft edit changes the whole amount due later, not a settled difference"
    );

    assert_eq!(
        inventory.variant_availability(&variant).await.unwrap(),
        10,
        "a draft has not taken stock, so confirming its edit must not move any"
    );
    let reloaded = orders.get(&draft.id).await.unwrap();
    assert_eq!(reloaded.items[0].quantity, 5);
    assert_eq!(reloaded.total, 5_000);
}

#[tokio::test]
async fn an_empty_draft_cannot_be_converted() {
    let (orders, _db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 2))
        .await
        .unwrap();
    let edit = orders
        .create_order_edit(&draft.id, quantity_edit(draft.items[0].id.as_str(), 0))
        .await
        .unwrap();
    orders
        .confirm_order_edit(&draft.id, &edit.id)
        .await
        .unwrap();

    let error = orders.convert_draft_order(&draft.id).await.unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
    assert_eq!(
        orders.get(&draft.id).await.unwrap().status,
        OrderStatus::Draft
    );
}

#[tokio::test]
async fn after_sales_creation_is_refused_on_a_draft() {
    let (orders, _db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 1))
        .await
        .unwrap();

    let error = orders
        .create_return(
            &draft.id,
            CreateReturnInput {
                reason: None,
                reason_id: None,
                refund_id: None,
                metadata: Default::default(),
                items: vec![ReturnItemInput {
                    order_item_id: draft.items[0].id.clone(),
                    quantity: 1,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn re_calculating_tax_rewrites_the_item_breakdown() {
    let (orders, _db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 1))
        .await
        .unwrap();
    let item_id = draft.items[0].id.clone();

    let updated = orders
        .recalculate_draft_taxes(
            &draft.id,
            Some(json!({"country_code": "FR", "province_code": "IDF"})),
            None,
            vec![DraftOrderTaxLineInput {
                order_item_id: item_id.clone(),
                name: "VAT".to_owned(),
                rate_bps: 1_900,
                amount: 190,
                is_tax_inclusive: false,
            }],
        )
        .await
        .unwrap();

    assert_eq!(updated.tax_total, 190);
    assert_eq!(updated.total, 1_190);
    assert_eq!(updated.items[0].tax_total, 190);
    assert_eq!(updated.items[0].total, 1_190);
    assert_eq!(updated.tax_lines.len(), 1);
    assert_eq!(updated.shipping_address["country_code"], "FR");

    // A tax-inclusive line keeps the price: the tax is already contained in
    // the subtotal and must only change the breakdown.
    let inclusive = orders
        .recalculate_draft_taxes(
            &draft.id,
            None,
            None,
            vec![DraftOrderTaxLineInput {
                order_item_id: item_id,
                name: "VAT".to_owned(),
                rate_bps: 1_900,
                amount: 100,
                is_tax_inclusive: true,
            }],
        )
        .await
        .unwrap();
    assert_eq!(inclusive.tax_total, 100);
    assert_eq!(inclusive.total, 1_000);
    assert_eq!(inclusive.items[0].tax_total, 100);
    assert_eq!(inclusive.items[0].total, 1_000);
    assert_eq!(inclusive.shipping_address["country_code"], "FR");
}

#[tokio::test]
async fn re_calculating_tax_on_a_placed_order_is_refused() {
    let (orders, _db) = service().await;
    let placed = orders.create_order(draft_input(None, 1)).await.unwrap();

    let error = orders
        .recalculate_draft_taxes(&placed.id, None, None, Vec::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_conversion_refuses_a_snapshot_an_edit_overtook() {
    let (orders, _db) = service().await;
    let draft = orders
        .create_draft_order(draft_input(None, 2))
        .await
        .unwrap();
    let priced = DraftConversionSnapshot::from_order(&draft);

    // An edit between pricing and conversion changes the line the tax and
    // stock inputs described.
    let edit = orders
        .create_order_edit(&draft.id, quantity_edit(draft.items[0].id.as_str(), 3))
        .await
        .unwrap();
    orders
        .confirm_order_edit(&draft.id, &edit.id)
        .await
        .unwrap();

    let error = orders
        .convert_priced_draft_order(&draft.id, &priced, None, Vec::new())
        .await
        .unwrap_err();
    assert!(
        matches!(error, fastcart_kernel::AppError::Conflict(_)),
        "{error:?}"
    );
    let reloaded = orders.get(&draft.id).await.unwrap();
    assert_eq!(reloaded.status, OrderStatus::Draft);
    assert_eq!(reloaded.items[0].quantity, 3);

    // Re-pricing the current lines converts the draft.
    let fresh = DraftConversionSnapshot::from_order(&reloaded);
    let converted = orders
        .convert_priced_draft_order(&draft.id, &fresh, None, Vec::new())
        .await
        .unwrap();
    assert_eq!(converted.status, OrderStatus::Pending);
}
