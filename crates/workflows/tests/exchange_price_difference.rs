//! The workflow prices replacement lines when it confirms after-sales records.
//!
//! The order module cannot read the pricing module's tables (see the
//! module-boundary guard), so it cannot price a replacement variant by itself.
//! The workflow is the cross-module orchestrator: it resolves the replacement
//! unit prices from `PricingService` and passes them into
//! `OrderService::accept_*`, which settles the difference on the order.
//!
//! A replacement variant with no price in the order's currency has no known
//! difference; the workflow accepts the record and settles nothing rather than
//! guessing.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_order::{
    ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateExchangeInput,
    CreateOrderInput, CreateOrderItemInput, ExchangeItemInput, Order, OrderService,
};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_workflows::AfterSalesWorkflow;

struct Fixture {
    workflow: AfterSalesWorkflow,
    orders: OrderService,
    pricing: PricingService,
}

async fn fixture() -> Fixture {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let workflow = AfterSalesWorkflow::new(
        db.clone(),
        PaymentService::with_registry(db.clone(), providers),
    );
    Fixture {
        orders: OrderService::new(db.clone()),
        pricing: PricingService::new(db),
        workflow,
    }
}

async fn create_order(orders: &OrderService, quantity: i64, variant_id: VariantId) -> Order {
    orders
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity,
                variant_id: Some(variant_id),
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000 * quantity,
            total: 1_000 * quantity,
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

async fn price_variant(pricing: &PricingService, variant_id: &VariantId, amount: i64) {
    pricing
        .create_price(CreatePriceInput::new(
            variant_id.as_str(),
            CurrencyCode::new("USD").unwrap(),
            amount,
        ))
        .await
        .unwrap();
}

/// The identity `create_order` guarantees and every later flow must preserve.
fn assert_order_identity(order: &Order) {
    let line_totals: i64 = order.items.iter().map(|item| item.total).sum();
    let adjustments: i64 = order.adjustments.iter().map(|entry| entry.amount).sum();
    assert_eq!(
        line_totals + order.shipping_total + adjustments,
        order.total,
        "orders.total must be the sum of line totals plus shipping plus adjustments"
    );
}

#[tokio::test]
async fn accepting_an_exchange_prices_the_replacement_from_the_pricing_module() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 2, original).await;
    price_variant(&fixture.pricing, &replacement, 1_500).await;

    let exchange = fixture
        .orders
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

    let result = fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();

    assert_eq!(result.exchange.difference_total, Some(500));
    assert_eq!(result.order.total, 2_500);

    // Replaying the acceptance must not book the difference twice.
    let replay = fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();
    assert_eq!(replay.exchange.difference_total, Some(500));
    let order = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_500);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_order_identity(&order);
}

#[tokio::test]
async fn accepting_a_claim_prices_its_replacements_from_the_pricing_module() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 2, original).await;
    price_variant(&fixture.pricing, &replacement, 1_200).await;

    let claim = fixture
        .orders
        .create_claim(
            &order.id,
            CreateClaimInput {
                reason: None,
                reason_id: None,
                metadata: Metadata::empty(),
                items: vec![ClaimItemInput {
                    order_item_id: order.items[0].id.clone(),
                    quantity: 2,
                    item_type: ClaimItemType::Replace,
                    replacement_variant_id: Some(replacement),
                    reason: None,
                }],
            },
        )
        .await
        .unwrap();

    // The claim has no refund lines, so this accepts it without moving money.
    let result = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, None)
        .await
        .unwrap();

    assert_eq!(result.claim.difference_total, Some(400));
    assert_eq!(result.order.total, 2_400);
    assert_eq!(result.order.adjustments.len(), 1, "{}", result.order.total);
    assert_order_identity(&result.order);

    let replay = fixture
        .workflow
        .refund_claim(&order.id, &claim.id, None)
        .await
        .unwrap();
    assert_eq!(replay.claim.difference_total, Some(400));
    assert_eq!(replay.order.total, 2_400);
    assert_eq!(replay.order.adjustments.len(), 1);

    // The admin route accepts claims through this entry point, which keeps
    // calling the order service for an already accepted claim: the order
    // service's own replay guard is what has to hold here.
    let accepted_again = fixture
        .workflow
        .accept_claim(&order.id, &claim.id)
        .await
        .unwrap();
    assert_eq!(accepted_again.status, ClaimStatus::Accepted);
    assert_eq!(accepted_again.difference_total, Some(400));
    let order = fixture.orders.get(&order.id).await.unwrap();
    assert_eq!(order.total, 2_400);
    assert_eq!(order.adjustments.len(), 1, "{:?}", order.adjustments);
    assert_order_identity(&order);
}

#[tokio::test]
async fn a_replacement_without_a_price_settles_nothing() {
    let fixture = fixture().await;
    let original = VariantId::new();
    let replacement = VariantId::new();
    let order = create_order(&fixture.orders, 2, original).await;

    let exchange = fixture
        .orders
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

    let result = fixture
        .workflow
        .accept_exchange(&order.id, &exchange.id)
        .await
        .unwrap();

    assert_eq!(result.exchange.difference_total, Some(0));
    assert_eq!(result.order.total, 2_000);
    assert!(
        result.order.adjustments.is_empty(),
        "{:?}",
        result.order.adjustments
    );
    assert_order_identity(&result.order);
}
