//! A replacement is priced for the customer doing the exchanging.
//!
//! Price lists can be scoped to a customer group. The settlement priced every
//! replacement with no group at all, so a member exchanging a line paid the
//! list price for a replacement their group price already covered — or was
//! credited as if the list price applied.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_customer::{CreateCustomerInput, CustomerService};
use fastcart_module_order::{
    CreateExchangeInput, CreateOrderInput, CreateOrderItemInput, ExchangeItemInput, Order,
    OrderService,
};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_workflows::AfterSalesWorkflow;

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

#[tokio::test]
async fn a_group_price_prices_the_replacement_of_a_group_member() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();

    let customers = CustomerService::new(db.clone());
    let customer = customers
        .create_customer(CreateCustomerInput {
            email: "vip@example.com".to_owned(),
            first_name: None,
            last_name: None,
            phone: None,
            metadata: Metadata::empty(),
        })
        .await
        .unwrap();
    let group = customers
        .create_group("VIP", Metadata::empty())
        .await
        .unwrap();
    customers
        .assign_group(&customer.id, group.id.as_str())
        .await
        .unwrap();

    let replacement = VariantId::new();
    let pricing = PricingService::new(db.clone());
    // The list price the replacement would cost anybody, and the group price
    // the member is entitled to.
    pricing
        .create_price(CreatePriceInput::new(replacement.as_str(), usd(), 1_500))
        .await
        .unwrap();
    pricing
        .create_price(
            CreatePriceInput::new(replacement.as_str(), usd(), 800)
                .with_customer_group(group.id.as_str()),
        )
        .await
        .unwrap();

    let orders = OrderService::new(db.clone());
    let original = VariantId::new();
    let order = orders
        .create_order(CreateOrderInput {
            customer_id: Some(customer.id.clone()),
            email: "vip@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                variant_id: Some(original),
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            total: 1_000,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap();

    let exchange = orders
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
                    quantity: 1,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let workflow = AfterSalesWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );
    workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();

    let order: Order = orders.get(&order.id).await.unwrap();
    // 800 (group price) - 1 000 (the line it replaces), not 1 500 - 1 000.
    assert_eq!(
        order
            .adjustments
            .iter()
            .map(|entry| entry.amount)
            .sum::<i64>(),
        -200,
        "the difference settles at the price the member is entitled to"
    );
    assert_eq!(order.total, 800);
}
