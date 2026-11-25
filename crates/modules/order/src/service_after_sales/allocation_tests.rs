use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use serde_json::json;

use crate::after_sales::{CreateOrderEditInput, OrderEditAction, OrderEditItemInput};
use crate::domain::{
    CreateAdjustmentInput, CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput,
};
use crate::service::OrderService;

/// A single item-scoped tax line, matching what checkout snapshots.
fn tax_line(amount: i64) -> CreateOrderTaxLineInput {
    item_tax_line(0, amount)
}

/// An item-scoped tax line for the item at `index`.
fn item_tax_line(index: usize, amount: i64) -> CreateOrderTaxLineInput {
    CreateOrderTaxLineInput {
        item_index: Some(index),
        shipping_option_id: None,
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: false,
    }
}

/// A shipping-scoped tax line, matching what checkout snapshots when the
/// shipping option is taxed.
fn shipping_tax_line(amount: i64) -> CreateOrderTaxLineInput {
    CreateOrderTaxLineInput {
        item_index: None,
        shipping_option_id: Some("ship_1".to_owned()),
        name: "Shipping tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: false,
    }
}

/// An item-scoped tax line whose amount is already inside the unit price.
fn inclusive_item_tax_line(index: usize, amount: i64) -> CreateOrderTaxLineInput {
    CreateOrderTaxLineInput {
        item_index: Some(index),
        shipping_option_id: None,
        name: "VAT".to_owned(),
        rate_bps: 1_900,
        amount,
        is_tax_inclusive: true,
    }
}

/// A shipping-tax adjustment, matching checkout and draft creation.
fn shipping_tax_adjustment(amount: i64) -> CreateAdjustmentInput {
    CreateAdjustmentInput {
        code: Some("shipping_tax".to_owned()),
        amount,
        ..CreateAdjustmentInput::default()
    }
}

#[tokio::test]
async fn confirming_a_removal_scales_order_level_allocations_proportionally() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 2,
                subtotal: Some(2_000),
                discount_total: Some(201),
                tax_total: Some(179),
                total: Some(1_978),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            adjustments: vec![CreateAdjustmentInput {
                code: Some("handling".to_owned()),
                amount: -101,
                ..CreateAdjustmentInput::default()
            }],
            subtotal: 2_000,
            discount_total: 201,
            shipping_total: 500,
            tax_total: 179,
            tax_lines: vec![tax_line(179)],
            total: 2_377,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("remove one".to_owned()),
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
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(edit.after_subtotal, 1_000);
    assert_eq!(edit.after_total, 1_377);

    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();

    // Retained subtotal is 1000/2000 of the original base.
    assert_eq!(reloaded.subtotal, 1_000);
    assert_eq!(reloaded.discount_total, 100);
    assert_eq!(reloaded.tax_total, 89);
    assert_eq!(reloaded.shipping_total, 500);
    assert_eq!(reloaded.adjustments.len(), 1);
    // floor(-101 * 1000 / 2000) = -51
    assert_eq!(reloaded.adjustments[0].amount, -51);
    assert_eq!(reloaded.total, 1_438);
    assert_eq!(confirmed.after_subtotal, 1_000);
    assert_eq!(confirmed.after_total, 1_438);
    assert_eq!(
        reloaded.total,
        reloaded.subtotal - reloaded.discount_total
            + reloaded.shipping_total
            + reloaded.tax_total
            + reloaded
                .adjustments
                .iter()
                .map(|value| value.amount)
                .sum::<i64>()
    );
    // The removal lands on the line: it keeps its row at the reduced
    // quantity and the order totals are the sums of the materialized lines.
    assert_eq!(reloaded.items[0].quantity, 1);
    assert_eq!(reloaded.items[0].subtotal, 1_000);
    assert_eq!(reloaded.items[0].discount_total, 100);
    assert_eq!(reloaded.items[0].tax_total, 89);
    assert_eq!(reloaded.items[0].total, 989);
    assert_eq!(reloaded.items[0].subtotal, reloaded.subtotal);
    assert_eq!(reloaded.items[0].discount_total, reloaded.discount_total);
    assert_eq!(reloaded.items[0].tax_total, reloaded.tax_total);
    // The tax breakdown follows the line, so it still adds up to the tax
    // the order persists.
    assert_eq!(
        reloaded
            .tax_lines
            .iter()
            .map(|line| line.amount)
            .sum::<i64>(),
        reloaded.tax_total
    );
}

#[tokio::test]
async fn additions_keep_explicit_amounts_and_do_not_scale_allocations() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
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

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: None,
                metadata: fastcart_kernel::Metadata::default(),
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
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();
    assert_eq!(reloaded.subtotal, 2_500);
    assert_eq!(reloaded.discount_total, 200);
    assert_eq!(reloaded.tax_total, 180);
    assert_eq!(reloaded.total, 2_980);
    assert_eq!(confirmed.after_total, 2_980);
}

#[tokio::test]
async fn order_tax_lines_follow_their_base_and_keep_adding_up() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
    // Two taxed lines plus tax levied on shipping: 151 floors to 75 when
    // the widget shrinks to one unit.
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![
                CreateOrderItemInput {
                    product_title: "Widget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    subtotal: Some(2_000),
                    tax_total: Some(151),
                    total: Some(2_151),
                    requires_shipping: true,
                    ..CreateOrderItemInput::default()
                },
                CreateOrderItemInput {
                    product_title: "Gadget".to_owned(),
                    unit_price: 1_000,
                    quantity: 2,
                    subtotal: Some(2_000),
                    tax_total: Some(100),
                    total: Some(2_100),
                    requires_shipping: true,
                    ..CreateOrderItemInput::default()
                },
            ],
            subtotal: 4_000,
            shipping_total: 500,
            tax_total: 281,
            tax_lines: vec![
                item_tax_line(0, 151),
                item_tax_line(1, 100),
                shipping_tax_line(30),
            ],
            total: 4_751,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let edit = service
        .create_order_edit(
            &order.id,
            CreateOrderEditInput {
                reason: Some("shrink and remove".to_owned()),
                metadata: fastcart_kernel::Metadata::default(),
                items: vec![
                    OrderEditItemInput {
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
                        is_tax_inclusive: None,
                    },
                    OrderEditItemInput {
                        order_item_id: Some(order.items[1].id.clone()),
                        action: OrderEditAction::Remove,
                        product_id: None,
                        variant_id: None,
                        product_title: None,
                        variant_title: None,
                        sku: None,
                        unit_price: None,
                        before_quantity: Some(2),
                        after_quantity: 0,
                        requires_shipping: None,
                        is_tax_inclusive: None,
                    },
                ],
            },
        )
        .await
        .unwrap();
    service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();

    // floor(151 / 2) = 75: the widget's tax follows its own line, and the
    // removed line's tax follows it to zero.
    assert_eq!(reloaded.items[0].quantity, 1);
    assert_eq!(reloaded.items[0].tax_total, 75);
    assert_eq!(reloaded.items[1].quantity, 0);
    assert_eq!(reloaded.items[1].tax_total, 0);
    let widget_tax = reloaded
        .tax_lines
        .iter()
        .find(|line| line.order_item_id.as_ref() == Some(&reloaded.items[0].id))
        .expect("the widget keeps its tax line");
    assert_eq!(widget_tax.amount, 75);
    let gadget_tax = reloaded
        .tax_lines
        .iter()
        .find(|line| line.order_item_id.as_ref() == Some(&reloaded.items[1].id))
        .expect("the removed line keeps its tax line");
    assert_eq!(gadget_tax.amount, 0);
    // Tax levied on shipping is preserved with the shipping.
    let shipping_tax = reloaded
        .tax_lines
        .iter()
        .find(|line| line.order_item_id.is_none())
        .expect("the shipping tax line must survive the edit");
    assert_eq!(shipping_tax.amount, 30);

    // The breakdown adds up to the order tax exactly, whatever the
    // per-line flooring did, and the order still covers its item taxes.
    assert_eq!(reloaded.tax_total, 105);
    assert_eq!(
        reloaded
            .tax_lines
            .iter()
            .map(|line| line.amount)
            .sum::<i64>(),
        reloaded.tax_total
    );
    let item_tax: i64 = reloaded.items.iter().map(|item| item.tax_total).sum();
    assert_eq!(item_tax, 75);
    assert!(item_tax <= reloaded.tax_total);
    assert_eq!(reloaded.subtotal, 1_000);
    assert_eq!(reloaded.total, 1_575);
}

#[tokio::test]
async fn shipping_tax_is_preserved_when_the_goods_shrink() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
    // 2x1000 at 19% exclusive plus 500 shipping taxed at 19%: item tax 380,
    // shipping tax 95, total 2975.
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "EUR".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 2,
                subtotal: Some(2_000),
                tax_total: Some(380),
                total: Some(2_380),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            adjustments: vec![shipping_tax_adjustment(95)],
            subtotal: 2_000,
            shipping_total: 500,
            tax_total: 475,
            tax_lines: vec![
                CreateOrderTaxLineInput {
                    rate_bps: 1_900,
                    ..item_tax_line(0, 380)
                },
                shipping_tax_line(95),
            ],
            total: 2_975,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let edit = service
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
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();

    assert_eq!(reloaded.items[0].subtotal, 1_000);
    assert_eq!(reloaded.items[0].tax_total, 190);
    assert_eq!(reloaded.items[0].total, 1_190);
    // The shipping and the tax on it do not follow the retained goods.
    assert_eq!(reloaded.adjustments.len(), 1);
    assert_eq!(reloaded.adjustments[0].amount, 95);
    assert_eq!(reloaded.tax_total, 285);
    assert_eq!(
        reloaded
            .tax_lines
            .iter()
            .map(|line| line.amount)
            .sum::<i64>(),
        reloaded.tax_total
    );
    // Total = retained line total + preserved shipping + preserved shipping
    // tax; every unit of `tax_total` is embedded in it.
    assert_eq!(reloaded.total, 1_785);
    assert_eq!(confirmed.difference_total, Some(-1_190));
}

#[tokio::test]
async fn growing_a_line_scales_its_tax() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "EUR".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                subtotal: Some(1_000),
                tax_total: Some(190),
                total: Some(1_190),
                requires_shipping: false,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            tax_total: 190,
            tax_lines: vec![CreateOrderTaxLineInput {
                rate_bps: 1_900,
                ..item_tax_line(0, 190)
            }],
            total: 1_190,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let edit = service
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
                    before_quantity: Some(1),
                    after_quantity: 2,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();

    assert_eq!(reloaded.items[0].subtotal, 2_000);
    assert_eq!(reloaded.items[0].tax_total, 380);
    assert_eq!(reloaded.items[0].total, 2_380);
    assert_eq!(reloaded.tax_total, 380);
    assert_eq!(reloaded.tax_lines[0].amount, 380);
    assert_eq!(reloaded.total, 2_380);
    assert_eq!(confirmed.difference_total, Some(1_190));
}

#[tokio::test]
async fn inclusive_tax_stays_inside_the_line_total_when_a_line_grows() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db);
    // 1190 per unit including 19% VAT: the tax is part of the price, so the
    // line total is the subtotal, not subtotal plus tax.
    let order = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "EUR".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_190,
                quantity: 1,
                subtotal: Some(1_190),
                tax_total: Some(190),
                total: Some(1_190),
                is_tax_inclusive: true,
                requires_shipping: false,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_190,
            tax_total: 190,
            tax_lines: vec![inclusive_item_tax_line(0, 190)],
            total: 1_190,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let edit = service
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
                    before_quantity: Some(1),
                    after_quantity: 2,
                    requires_shipping: None,
                    is_tax_inclusive: None,
                }],
            },
        )
        .await
        .unwrap();
    let confirmed = service
        .confirm_order_edit(&order.id, &edit.id)
        .await
        .unwrap();
    let reloaded = service.get(&order.id).await.unwrap();

    assert_eq!(reloaded.subtotal, 2_380);
    assert_eq!(reloaded.items[0].tax_total, 380);
    assert_eq!(reloaded.items[0].total, 2_380);
    assert_eq!(reloaded.tax_total, 380);
    assert_eq!(reloaded.total, 2_380);
    assert_eq!(confirmed.difference_total, Some(1_190));
}
