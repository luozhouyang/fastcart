//! The difference an accepted record owes the customer can be paid back.
//!
//! Accepting an exchange or a claim settles the difference on the order. When
//! the replacement is cheaper, that difference is money the customer is owed,
//! and until now nothing paid it: the order total shrank and the customer's
//! card was never credited. Paying it out stays the merchant's decision — it is
//! a separate call, not a side effect of accepting — but it must be one call,
//! idempotent, and bounded by what the order still holds.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    CreateExchangeInput, CreateOrderInput, CreateOrderItemInput, ExchangeItemInput, Order,
    OrderService,
};
use fastcart_module_payment::{
    CreatePaymentSessionInput, ManualProvider, PaymentService, ProviderRegistry,
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

/// Captures the whole order on the sandbox provider, as checkout would.
async fn capture_payment(payments: &PaymentService, order: &Order) {
    let session = payments
        .create_session(CreatePaymentSessionInput {
            order_id: Some(order.id.clone()),
            provider: "manual".to_owned(),
            amount: order.total,
            currency_code: order.currency_code.clone(),
            ..CreatePaymentSessionInput::default()
        })
        .await
        .unwrap();
    payments.capture(session.id.as_str()).await.unwrap();
}

/// Exchanges the order's only line for a replacement priced at `unit_price`,
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

#[tokio::test]
async fn a_cheaper_replacement_refunds_what_the_customer_was_owed() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    capture_payment(&fixture.payments, &order).await;
    let exchange = exchange_for(&fixture, &order, 700).await;
    assert_eq!(exchange.difference_total, Some(-300));

    let refunded = fixture
        .workflow
        .refund_exchange_difference(&order.id, &exchange.id)
        .await
        .unwrap();

    let order_refund = refunded
        .order_refund
        .expect("the difference is recorded as an order refund");
    assert_eq!(order_refund.amount, 300);
    let provider_refund = refunded
        .provider_refund
        .expect("the money is sent back through the provider");
    assert_eq!(provider_refund.amount, 300);
    assert_eq!(refunded.order.refunds.len(), 1);
}

#[tokio::test]
async fn refunding_the_same_difference_twice_refunds_once() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    capture_payment(&fixture.payments, &order).await;
    let exchange = exchange_for(&fixture, &order, 700).await;

    fixture
        .workflow
        .refund_exchange_difference(&order.id, &exchange.id)
        .await
        .unwrap();
    let replayed = fixture
        .workflow
        .refund_exchange_difference(&order.id, &exchange.id)
        .await
        .unwrap();

    assert_eq!(replayed.order.refunds.len(), 1);
    assert_eq!(
        replayed
            .order
            .refunds
            .iter()
            .map(|refund| refund.amount)
            .sum::<i64>(),
        300
    );
}

#[tokio::test]
async fn a_dearer_replacement_leaves_nothing_to_refund() {
    let fixture = fixture().await;
    let order = order(&fixture.orders, 1_000).await;
    capture_payment(&fixture.payments, &order).await;
    let exchange = exchange_for(&fixture, &order, 1_400).await;
    assert_eq!(exchange.difference_total, Some(400));

    let error = fixture
        .workflow
        .refund_exchange_difference(&order.id, &exchange.id)
        .await
        .unwrap_err();

    assert!(
        matches!(&error, fastcart_kernel::AppError::Conflict(_)),
        "unexpected error: {error:?}"
    );
    let order = fixture.orders.get(&order.id).await.unwrap();
    assert!(order.refunds.is_empty(), "{:?}", order.refunds);
}
