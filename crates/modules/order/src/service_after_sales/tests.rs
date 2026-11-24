use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{OrderId, PaymentId, RefundId, StoreId, VariantId};
use serde_json::json;

use crate::after_sales::{
    ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateExchangeInput,
    CreateOrderEditInput, CreateReturnInput, ExchangeItemInput, ExchangeStatus, OrderEditAction,
    OrderEditItemInput, OrderEditStatus, ReceiveReturnInput, ReceiveReturnItemInput,
    ReturnItemInput, ReturnStatus, WriteOffReturnInput,
};
use crate::service::OrderService;
/// A single item-scoped tax line, matching what checkout snapshots.
fn tax_line(amount: i64) -> CreateOrderTaxLineInput {
    CreateOrderTaxLineInput {
        item_index: Some(0),
        shipping_option_id: None,
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: false,
    }
}

use crate::domain::{
    CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput, Order, OrderStatus,
    RefundInput,
};

async fn service() -> OrderService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    OrderService::new(db)
}

async fn order(service: &OrderService, quantity: i64) -> Order {
    let unit_price = 1_000;
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
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

fn return_input(order: &Order, quantity: i64) -> CreateReturnInput {
    CreateReturnInput {
        reason: Some("damaged".to_owned()),
        refund_id: None,
        metadata: fastcart_kernel::Metadata::default(),
        items: vec![ReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity,
            reason: Some("broken".to_owned()),
        }],
    }
}

#[tokio::test]
async fn return_quantity_cannot_exceed_purchased() {
    let service = service().await;
    let order = order(&service, 2).await;

    let error = service
        .create_return(&order.id, return_input(&order, 3))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let created = service
        .create_return(&order.id, return_input(&order, 2))
        .await
        .unwrap();
    assert_eq!(created.status, ReturnStatus::Draft);

    let error = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn return_lifecycle_is_idempotent_and_quantity_checked() {
    let service = service().await;
    let order = order(&service, 2).await;
    let created = service
        .create_return(&order.id, return_input(&order, 2))
        .await
        .unwrap();

    let requested = service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    assert_eq!(requested.status, ReturnStatus::Requested);
    let again = service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    assert_eq!(again.status, ReturnStatus::Requested);

    // Receiving more than requested is rejected.
    let error = service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 3,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let partial = service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(partial.status, ReturnStatus::PartiallyReceived);
    assert_eq!(partial.items[0].received_quantity, 1);

    let received = service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(received.status, ReturnStatus::Received);
    assert_eq!(received.items[0].received_quantity, 2);

    // Repeating the receive after completion is a no-op.
    let again = service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();
    assert_eq!(again.status, ReturnStatus::Received);

    let written_off = service
        .write_off_return(&order.id, &created.id, WriteOffReturnInput::default())
        .await
        .unwrap();
    assert_eq!(written_off.status, ReturnStatus::WrittenOff);
    assert_eq!(written_off.items[0].written_off_quantity, 2);

    let error = service
        .cancel_return(&order.id, &created.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn write_off_cannot_exceed_received() {
    let service = service().await;
    let order = order(&service, 3).await;
    let created = service
        .create_return(&order.id, return_input(&order, 3))
        .await
        .unwrap();
    service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    service
        .receive_return(
            &order.id,
            &created.id,
            ReceiveReturnInput {
                items: vec![ReceiveReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                }],
            },
        )
        .await
        .unwrap();

    let error = service
        .write_off_return(
            &order.id,
            &created.id,
            WriteOffReturnInput {
                items: vec![crate::after_sales::WriteOffReturnItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

/// A write-off reversal must not drive `stocked_quantity` negative. The
/// compensating UPDATE carries the same `stocked_quantity >= quantity`
/// guard as inventory commits, so a level that no longer holds the restock
/// yields a conflict and leaves stock untouched.
#[tokio::test]
async fn write_off_reversal_rejects_insufficient_stock() {
    let service = service().await;
    let variant_id = VariantId::new();
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                variant_id: Some(variant_id.clone()),
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    let created = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    service
        .request_return(&order.id, &created.id)
        .await
        .unwrap();
    service
        .receive_return(&order.id, &created.id, ReceiveReturnInput::default())
        .await
        .unwrap();

    // Receiving normally restocks through InventoryService. Simulate that
    // restock movement, but leave the destination level already drained.
    let pool = service.database().pool();
    sqlx::query(
        "INSERT INTO inventory_locations \
             (id, name, address, metadata, created_at, updated_at) \
             VALUES ('loc_test', 'Test location', '{}', '{}', 0, 0)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_items \
             (id, sku, variant_id, metadata, created_at, updated_at) \
             VALUES ('item_test', NULL, ?, '{}', 0, 0)",
    )
    .bind(variant_id.as_str())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inventory_levels \
             (id, inventory_item_id, location_id, stocked_quantity, reserved_quantity, \
              incoming_quantity, created_at, updated_at) \
             VALUES ('level_test', 'item_test', 'loc_test', 0, 0, 0, 0, 0)",
    )
    .execute(pool)
    .await
    .unwrap();
    let restock_reference = format!(
        "return-restock:{}:{}:1",
        created.id.as_str(),
        order.items[0].id.as_str()
    );
    sqlx::query(
        "INSERT INTO inventory_movements \
             (id, inventory_item_id, location_id, quantity_delta, reason, reference, \
              actor_id, metadata, created_at) \
             VALUES ('mv_restock', 'item_test', 'loc_test', 1, 'return_restock', ?, NULL, '{}', 1)",
    )
    .bind(&restock_reference)
    .execute(pool)
    .await
    .unwrap();

    let write_off = WriteOffReturnInput {
        items: vec![crate::after_sales::WriteOffReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity: 1,
        }],
    };
    let error = service
        .write_off_return(&order.id, &created.id, write_off.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    // The failed reversal rolled back: stock, movement and return item are
    // unchanged.
    let stock: i64 =
        sqlx::query_scalar("SELECT stocked_quantity FROM inventory_levels WHERE id = 'level_test'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stock, 0);
    let reversals: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_movements WHERE reason = 'return_writeoff'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(reversals, 0);
    let written_off: i64 = sqlx::query_scalar(
        "SELECT written_off_quantity FROM order_return_items WHERE return_id = ?",
    )
    .bind(created.id.as_str())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(written_off, 0);

    // With the restocked unit present the same call succeeds.
    sqlx::query("UPDATE inventory_levels SET stocked_quantity = 1 WHERE id = 'level_test'")
        .execute(pool)
        .await
        .unwrap();
    let written_off = service
        .write_off_return(&order.id, &created.id, write_off)
        .await
        .unwrap();
    assert_eq!(written_off.status, ReturnStatus::WrittenOff);
    assert_eq!(written_off.items[0].written_off_quantity, 1);
    let stock: i64 =
        sqlx::query_scalar("SELECT stocked_quantity FROM inventory_levels WHERE id = 'level_test'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stock, 0);
}

#[tokio::test]
async fn claims_respect_returned_quantities_and_transitions() {
    let service = service().await;
    let order = order(&service, 4).await;
    let returned = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    service
        .request_return(&order.id, &returned.id)
        .await
        .unwrap();

    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 3,
                    item_type: ClaimItemType::Refund,
                    replacement_variant_id: None,
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.status, ClaimStatus::Requested);

    let error = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
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
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let accepted = service
        .accept_claim(&order.id, &claim.id, &[])
        .await
        .unwrap();
    assert_eq!(accepted.status, ClaimStatus::Accepted);
    let again = service
        .accept_claim(&order.id, &claim.id, &[])
        .await
        .unwrap();
    assert_eq!(again.status, ClaimStatus::Accepted);

    let error = service
        .cancel_claim(&order.id, &claim.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn exchanges_track_inbound_and_outbound_items() {
    let service = service().await;
    let order = order(&service, 2).await;
    let variant = VariantId::new();

    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: Some("wrong size".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(variant.clone()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(exchange.status, ExchangeStatus::Requested);
    assert_eq!(exchange.inbound_items.len(), 1);
    assert_eq!(exchange.outbound_items.len(), 1);
    assert_eq!(
        exchange.outbound_items[0].replacement_variant_id.as_ref(),
        Some(&variant)
    );

    let error = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[])
        .await
        .unwrap();
    assert_eq!(accepted.status, ExchangeStatus::Accepted);
}

#[tokio::test]
async fn order_edits_recalculate_totals_and_materialize_the_line_changes() {
    let service = service().await;
    let order = order(&service, 2).await;

    let added = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("add gift".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: Some("GIFT-1".to_owned()),
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(added.status, OrderEditStatus::Draft);
    assert_eq!(added.before_total, 2_000);
    assert_eq!(added.after_total, 2_500);
    assert_eq!(added.items[0].before_quantity, 0);
    assert_eq!(added.items[0].after_quantity, 1);

    let confirmed = service
        .confirm_order_edit(&order.id, &added.id)
        .await
        .unwrap();
    assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.total, 2_500);
    // The edit materializes: the added line exists as an order item and the
    // order totals are the sums of the materialized lines.
    assert_eq!(reloaded.items.len(), 2);
    let added_line = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be materialized");
    assert_eq!(added_line.quantity, 1);
    assert_eq!(added_line.unit_price, 500);
    assert_eq!(added_line.subtotal, 500);
    assert_eq!(added_line.total, 500);
    let purchased = reloaded
        .items
        .iter()
        .find(|item| item.sku.is_none())
        .expect("the purchased line must survive the edit");
    assert_eq!(purchased.quantity, 2);
    assert_eq!(purchased.unit_price, 1_000);
    assert_eq!(purchased.subtotal, 2_000);
    assert_eq!(
        reloaded.items.iter().map(|item| item.subtotal).sum::<i64>(),
        reloaded.subtotal
    );
    assert_eq!(
        reloaded.items.iter().map(|item| item.total).sum::<i64>(),
        reloaded.total
    );

    // Re-confirming is idempotent and must not duplicate the line.
    let again = service
        .confirm_order_edit(&order.id, &added.id)
        .await
        .unwrap();
    assert_eq!(again.status, OrderEditStatus::Confirmed);
    assert_eq!(service.get(&order.id).await.unwrap().items.len(), 2);

    let error = service
        .cancel_order_edit(&order.id, &added.id)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn order_edits_validate_before_quantities_and_overflow() {
    let service = service().await;
    let order = order(&service, 2).await;

    let error = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: Some(order.items[0].id.clone()),
                    action: OrderEditAction::Remove,
                    product_id: None,
                    variant_id: None,
                    product_title: None,
                    variant_title: None,
                    sku: None,
                    unit_price: None,
                    before_quantity: Some(1),
                    after_quantity: 0,
                    requires_shipping: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let update = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: Some(order.items[0].id.clone()),
                    action: OrderEditAction::Update,
                    product_id: None,
                    variant_id: None,
                    product_title: None,
                    variant_title: None,
                    sku: None,
                    unit_price: None,
                    before_quantity: Some(2),
                    after_quantity: 1,
                    requires_shipping: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(update.after_total, 1_000);
    let confirmed = service
        .confirm_order_edit(&order.id, &update.id)
        .await
        .unwrap();
    assert_eq!(confirmed.status, OrderEditStatus::Confirmed);
    assert_eq!(service.get(&order.id).await.unwrap().total, 1_000);
}

#[tokio::test]
async fn confirming_an_edit_keeps_all_order_totals_consistent() {
    let service = service().await;
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 2,
                subtotal: Some(2_000),
                discount_total: Some(200),
                tax_total: Some(180),
                total: Some(1_980),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 2_000,
            discount_total: 200,
            shipping_total: 500,
            tax_total: 180,
            tax_lines: vec![tax_line(180)],
            total: 2_480,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();
    assert_eq!(order.total, 2_480);

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("add gift".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![OrderEditItemInput {
                    order_item_id: None,
                    action: OrderEditAction::Add,
                    product_id: None,
                    variant_id: None,
                    product_title: Some("Gift card".to_owned()),
                    variant_title: None,
                    sku: Some("GIFT-1".to_owned()),
                    unit_price: Some(500),
                    before_quantity: None,
                    after_quantity: 1,
                    requires_shipping: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(edit.after_subtotal, 2_500);
    assert_eq!(edit.after_total, 2_980);

    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    assert_eq!(confirmed.after_subtotal, 2_500);
    assert_eq!(confirmed.after_total, 2_980);

    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.discount_total, 200);
    assert_eq!(reloaded.shipping_total, 500);
    assert_eq!(reloaded.tax_total, 180);
    assert_eq!(reloaded.total, 2_980);
    assert_eq!(
        reloaded.total,
        reloaded.subtotal - reloaded.discount_total + reloaded.shipping_total + reloaded.tax_total
    );
    // The added line is materialized and every order total is the sum of
    // the materialized lines.
    assert_eq!(reloaded.items.len(), 2);
    let purchased = reloaded
        .items
        .iter()
        .find(|item| item.sku.is_none())
        .expect("the purchased line must survive the edit");
    assert_eq!(purchased.quantity, 2);
    assert_eq!(purchased.subtotal, 2_000);
    assert_eq!(purchased.discount_total, 200);
    assert_eq!(purchased.tax_total, 180);
    assert_eq!(purchased.total, 1_980);
    let added_line = reloaded
        .items
        .iter()
        .find(|item| item.sku.as_deref() == Some("GIFT-1"))
        .expect("the added line must be materialized");
    assert_eq!(added_line.subtotal, 500);
    assert_eq!(added_line.total, 500);
    assert_eq!(
        reloaded.items.iter().map(|item| item.subtotal).sum::<i64>(),
        reloaded.subtotal
    );
    assert_eq!(
        reloaded
            .items
            .iter()
            .map(|item| item.discount_total)
            .sum::<i64>(),
        reloaded.discount_total
    );
    assert_eq!(
        reloaded
            .items
            .iter()
            .map(|item| item.tax_total)
            .sum::<i64>(),
        reloaded.tax_total
    );
    assert_eq!(
        reloaded.items.iter().map(|item| item.total).sum::<i64>() + reloaded.shipping_total,
        reloaded.total
    );
}

#[tokio::test]
async fn idempotent_order_refunds_do_not_write_twice() {
    let service = service().await;
    let order = order(&service, 2).await;
    let input = RefundInput {
        payment_id: PaymentId::new(),
        amount: 500,
        currency: "USD".to_owned(),
        reason: Some("return".to_owned()),
    };

    let (first_order, first_id) = service
        .add_refund_idempotent(&order.id, input.clone(), "after-sales:return-refund:ret_1")
        .await
        .unwrap();
    let (second_order, second_id) = service
        .add_refund_idempotent(&order.id, input, "after-sales:return-refund:ret_1")
        .await
        .unwrap();

    assert_eq!(first_id, second_id);
    assert_eq!(first_order.refunds.len(), 1);
    assert_eq!(second_order.refunds.len(), 1);
    assert_eq!(second_order.refunds[0].id, first_id);
    assert_eq!(second_order.refunds[0].amount, 500);
}

#[tokio::test]
async fn after_sales_refund_links_are_idempotent() {
    let service = service().await;
    let order = order(&service, 2).await;

    let created = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap();
    let refund = RefundId::new();
    let linked = service
        .attach_return_refund(&order.id, &created.id, &refund)
        .await
        .unwrap();
    assert_eq!(linked.refund_id.as_ref(), Some(&refund));
    let linked_again = service
        .attach_return_refund(&order.id, &created.id, &refund)
        .await
        .unwrap();
    assert_eq!(linked_again.refund_id.as_ref(), Some(&refund));
    let other = RefundId::new();
    assert!(matches!(
        service
            .attach_return_refund(&order.id, &created.id, &other)
            .await,
        Err(fastcart_kernel::AppError::Conflict(_))
    ));

    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
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
    let claim_refund = RefundId::new();
    let linked_claim = service
        .attach_claim_refund(&order.id, &claim.id, &claim_refund)
        .await
        .unwrap();
    assert_eq!(linked_claim.refund_id.as_ref(), Some(&claim_refund));
    let linked_claim_again = service
        .attach_claim_refund(&order.id, &claim.id, &claim_refund)
        .await
        .unwrap();
    assert_eq!(linked_claim_again.refund_id.as_ref(), Some(&claim_refund));
}

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

#[tokio::test]
async fn a_claimed_unit_cannot_also_be_returned() {
    let service = service().await;
    let order = order(&service, 1).await;
    service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
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

    let error = service
        .create_return(&order.id, return_input(&order, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn an_exchanged_inbound_unit_cannot_also_be_claimed() {
    let service = service().await;
    let order = order(&service, 1).await;
    service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let error = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
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
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
