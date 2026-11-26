//! Each replacement line carries the difference it settled.
//!
//! The record's `difference_total` says what the customer owes or is owed, but
//! not which replacement moved the money. Writing each line's share next to the
//! total keeps the two consistent by construction — the total is the sum of the
//! lines that settled one — and lets a refund or a charge name the line it is
//! for.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{LineItemId, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    ClaimItemInput, ClaimItemType, CreateClaimInput, CreateExchangeInput, CreateOrderInput,
    CreateOrderItemInput, ExchangeItemInput, Order, OrderService, ReplacementUnitPrice,
};

async fn service() -> (StoreDatabase, OrderService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone());
    (db, service)
}

async fn order(service: &OrderService, lines: i64) -> Order {
    let items: Vec<CreateOrderItemInput> = (0..lines)
        .map(|index| CreateOrderItemInput {
            product_title: format!("Widget {index}"),
            unit_price: 1_000,
            quantity: 1,
            ..CreateOrderItemInput::default()
        })
        .collect();
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items,
            subtotal: 1_000 * lines,
            total: 1_000 * lines,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

fn price(order_item_id: &LineItemId, unit_price: i64) -> ReplacementUnitPrice {
    ReplacementUnitPrice {
        order_item_id: order_item_id.clone(),
        unit_price,
    }
}

#[tokio::test]
async fn every_priced_replacement_line_carries_its_own_difference() {
    let (_db, service) = service().await;
    let order = order(&service, 2).await;
    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: order
                    .items
                    .iter()
                    .map(|item| ExchangeItemInput {
                        order_item_id: item.id.clone(),
                        quantity: 1,
                        replacement_variant_id: None,
                        reason: None,
                    })
                    .collect(),
                outbound_items: order
                    .items
                    .iter()
                    .map(|item| ExchangeItemInput {
                        order_item_id: item.id.clone(),
                        quantity: 1,
                        replacement_variant_id: Some(VariantId::new()),
                        reason: None,
                    })
                    .collect(),
            },
        )
        .await
        .unwrap();

    let accepted = service
        .accept_exchange(
            &order.id,
            &exchange.id,
            &[
                price(&order.items[0].id, 1_200),
                price(&order.items[1].id, 900),
            ],
        )
        .await
        .unwrap();

    let differences: Vec<Option<i64>> = accepted
        .outbound_items
        .iter()
        .map(|item| item.difference)
        .collect();
    assert_eq!(
        differences,
        vec![Some(200), Some(-100)],
        "each replacement line records what it settled"
    );
    assert!(
        accepted
            .inbound_items
            .iter()
            .all(|item| item.difference.is_none()),
        "inbound lines settle nothing: {:?}",
        accepted.inbound_items
    );
    assert_eq!(
        accepted.difference_total,
        Some(100),
        "the total is what the lines add up to"
    );
}

#[tokio::test]
async fn a_replacement_without_a_price_settles_no_line_difference() {
    let (_db, service) = service().await;
    let order = order(&service, 2).await;
    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: order
                    .items
                    .iter()
                    .map(|item| ExchangeItemInput {
                        order_item_id: item.id.clone(),
                        quantity: 1,
                        replacement_variant_id: None,
                        reason: None,
                    })
                    .collect(),
                outbound_items: order
                    .items
                    .iter()
                    .map(|item| ExchangeItemInput {
                        order_item_id: item.id.clone(),
                        quantity: 1,
                        replacement_variant_id: Some(VariantId::new()),
                        reason: None,
                    })
                    .collect(),
            },
        )
        .await
        .unwrap();

    // Only the first replacement has a price, so only its line settles one.
    let accepted = service
        .accept_exchange(&order.id, &exchange.id, &[price(&order.items[0].id, 1_200)])
        .await
        .unwrap();

    assert_eq!(
        accepted
            .outbound_items
            .iter()
            .map(|item| item.difference)
            .collect::<Vec<_>>(),
        vec![Some(200), None]
    );
    assert_eq!(accepted.difference_total, Some(200));
}

#[tokio::test]
async fn a_claim_replacement_carries_its_difference_too() {
    let (_db, service) = service().await;
    let order = order(&service, 1).await;
    let claim = service
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: None,
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

    let accepted = service
        .accept_claim(&order.id, &claim.id, &[price(&order.items[0].id, 700)])
        .await
        .unwrap();

    assert_eq!(
        accepted.items[0].difference,
        Some(-300),
        "the replaced line records what it settled"
    );
    assert_eq!(accepted.difference_total, Some(-300));
}
