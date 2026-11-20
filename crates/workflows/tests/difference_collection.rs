//! The difference a dearer replacement leaves the customer owing is payable.
//!
//! Accepting an exchange or a claim settles the difference on the order: the
//! order total grows by what the replacement costs more. That money was
//! recorded and then stranded — nothing created anything to pay it with.
//! Collecting it raises the order's payment collection by the difference and
//! opens a session for the increase, so the customer pays it the way they pay
//! anything else. Nothing is charged by the call itself.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    CreateExchangeInput, CreateOrderInput, CreateOrderItemInput, ExchangeItemInput, Order,
    OrderService,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, PaymentCollectionStatus, PaymentService,
    PaymentStatus, ProviderRegistry,
};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_workflows::AfterSalesWorkflow;

struct Fixture {
    workflow: AfterSalesWorkflow,
    orders: OrderService,
    pricing: PricingService,
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
        workflow,
        orders: OrderService::new(db.clone()),
        pricing: PricingService::new(db.clone()),
        payments: PaymentService::with_registry(db, providers),
    }
}

async fn order(orders: &OrderService, unit_price: i64) -> Order {
    orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity: 1,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price,
            total: unit_price,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

/// Exchanges the order's only line for a replacement priced at `unit_price`
/// and accepts it, settling the difference on the order.
async fn exchange_for(
    fixture: &Fixture,
    order: &Order,
    unit_price: i64,
) -> fastcart_module_order::OrderExchange {
    let replacement = VariantId::new();
    fixture
        .pricing
        .create_price(CreatePriceInput::new(
            replacement.as_str(),
            CurrencyCode::new("USD").unwrap(),
            unit_price,
        ))
        .await
        .unwrap();
    let exchange = fixture
        .orders
        .create_exchange(
            &order.id,
            CreateExchangeInput {
                reason: None,
                metadata: Metadata::empty(),
                inbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: None,
                    reason: None,
                }],
                outbound_items: vec![ExchangeItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 1,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();
    fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();
    fixture
        .orders
        .get_exchange(&order.id, &exchange.id)
        .await
        .unwrap()
}

/// Pays the order through a collection of its own, as a checkout would.
async fn collection_paid_in_full(fixture: &Fixture, order: &Order) {
    let collection = fixture
        .payments
        .create_collection_for_order(&order.id, order.total, "USD")
        .await
        .unwrap();
    let session = fixture
        .payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: "USD".to_owned(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    fixture
        .payments
        .attach_session_to_collection(collection.id.as_str(), session.id.as_str())
        .await
        .unwrap();
    fixture.payments.capture(session.id.as_str()).await.unwrap();
}

#[tokio::test]
async fn a_dearer_replacement_becomes_a_payable_on_the_order() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    let exchange = exchange_for(&fixture, &order, 1_400).await;
    assert_eq!(exchange.difference_total, Some(400));

    let collected = fixture
        .workflow
        .collect_exchange_difference(&order.id, &exchange.id, "manual")
        .await
        .unwrap();

    assert_eq!(collected.collection.amount, 400);
    assert_eq!(
        collected.session.amount, 400,
        "the session asks for exactly the difference"
    );
    assert_eq!(collected.session.status, PaymentStatus::Pending);
    let attached = fixture
        .payments
        .list_collection_sessions(collected.collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(attached.len(), 1);
}

#[tokio::test]
async fn collecting_a_difference_tops_up_the_collection_the_order_already_has() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    collection_paid_in_full(&fixture, &order).await;
    let exchange = exchange_for(&fixture, &order, 1_400).await;

    let collected = fixture
        .workflow
        .collect_exchange_difference(&order.id, &exchange.id, "manual")
        .await
        .unwrap();

    assert_eq!(
        collected.collection.amount, 1_400,
        "the order owes the difference on top of what it was paid"
    );
    assert_eq!(
        fixture
            .payments
            .collection_status(collected.collection.id.as_str())
            .await
            .unwrap(),
        PaymentCollectionStatus::PartiallyCaptured,
        "the increase is not paid yet"
    );
    let attached = fixture
        .payments
        .list_collection_sessions(collected.collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(
        attached.iter().map(|session| session.amount).sum::<i64>(),
        1_400,
        "the sessions now cover the raised amount"
    );
}

#[tokio::test]
async fn collecting_the_same_difference_twice_opens_one_session() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    let exchange = exchange_for(&fixture, &order, 1_400).await;

    let first = fixture
        .workflow
        .collect_exchange_difference(&order.id, &exchange.id, "manual")
        .await
        .unwrap();
    let replayed = fixture
        .workflow
        .collect_exchange_difference(&order.id, &exchange.id, "manual")
        .await
        .unwrap();

    assert_eq!(replayed.session.id, first.session.id);
    assert_eq!(
        replayed.collection.amount, 400,
        "a replay must not raise the amount twice"
    );
    let attached = fixture
        .payments
        .list_collection_sessions(first.collection.id.as_str())
        .await
        .unwrap();
    assert_eq!(attached.len(), 1);
}

#[tokio::test]
async fn a_cheaper_replacement_leaves_nothing_to_collect() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    let exchange = exchange_for(&fixture, &order, 700).await;
    assert_eq!(exchange.difference_total, Some(-300));

    let error = fixture
        .workflow
        .collect_exchange_difference(&order.id, &exchange.id, "manual")
        .await
        .unwrap_err();

    assert!(
        matches!(&error, fastcart_kernel::AppError::Conflict(_)),
        "unexpected error: {error:?}"
    );
}
