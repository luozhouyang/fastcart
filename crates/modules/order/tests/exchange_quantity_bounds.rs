//! A replacement stands in for the units the customer sends back.
//!
//! The difference settlement charges a replacement at the price of the line it
//! replaces: `(replacement price - replaced price) * quantity`. That identity
//! only holds while the exchange ships no more units than it takes back. An
//! outbound line with no inbound line to stand in for, or one that outnumbers
//! it, would hand the extra units over at the *difference* instead of at their
//! own price — the customer would keep a replacement they were never charged
//! for.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    CreateExchangeInput, CreateOrderInput, CreateOrderItemInput, ExchangeItemInput, Order,
    OrderService,
};

async fn service() -> (StoreDatabase, OrderService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = OrderService::new(db.clone());
    (db, service)
}

/// Places an order with `quantities.len()` lines, each priced at 1 000.
async fn order(service: &OrderService, quantities: &[i64]) -> Order {
    let items: Vec<CreateOrderItemInput> = quantities
        .iter()
        .enumerate()
        .map(|(index, quantity)| CreateOrderItemInput {
            product_title: format!("Widget {index}"),
            unit_price: 1_000,
            quantity: *quantity,
            ..CreateOrderItemInput::default()
        })
        .collect();
    let subtotal: i64 = quantities.iter().map(|quantity| quantity * 1_000).sum();
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items,
            subtotal,
            total: subtotal,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

async fn exchange_count(db: &StoreDatabase) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM order_exchanges")
        .fetch_one(db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_exchange_cannot_ship_more_replacements_than_it_takes_back() {
    let (db, service) = service().await;
    let order = order(&service, &[2]).await;

    let error = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::Conflict(message) if message.contains("replacement units")),
        "unexpected error: {error:?}"
    );
    assert_eq!(
        exchange_count(&db).await,
        0,
        "a rejected exchange must not leave a record behind"
    );
}

#[tokio::test]
async fn an_exchange_cannot_replace_a_line_it_does_not_take_back() {
    let (_db, service) = service().await;
    let order = order(&service, &[1, 1]).await;

    let error = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                // Nothing of the second line is coming back, so nothing can
                // stand in for it.
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[1].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(VariantId::new()),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap_err();

    assert!(
        matches!(&error, AppError::Conflict(message) if message.contains("replacement units")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn a_replacement_may_match_what_came_back_line_by_line() {
    let (_db, service) = service().await;
    let order = order(&service, &[2, 2]).await;

    // Each outbound line takes the place of what came back on its own line.
    let exchange = service
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                inbound_items: vec![
                    ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 2,
                        replacement_variant_id: None,
                        reason: None,
                    },
                    ExchangeItemInput {
                        order_item_id: order.items[1].id.clone(),
                        quantity: 1,
                        replacement_variant_id: None,
                        reason: None,
                    },
                ],
                outbound_items: vec![
                    ExchangeItemInput {
                        order_item_id: order.items[0].id.clone(),
                        quantity: 2,
                        replacement_variant_id: Some(VariantId::new()),
                        reason: None,
                    },
                    ExchangeItemInput {
                        order_item_id: order.items[1].id.clone(),
                        quantity: 1,
                        replacement_variant_id: Some(VariantId::new()),
                        reason: None,
                    },
                ],
            },
        )
        .await
        .unwrap();

    assert_eq!(exchange.outbound_items.len(), 2);
}
