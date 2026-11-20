//! Order- and shipping-target promotions through the cart workflow.
//!
//! An order-target promotion discounts the item subtotal plus shipping; the
//! application has no line, so the cart workflow records it as a cart-level
//! discount, which is the path `allocate_cart_discounts` and checkout already
//! understand. The cart total and the order placed from it must agree, and an
//! order-target promotion must not squeeze the coupon rows off the cart.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::CreateCartInput;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_payment::{ManualProvider, PaymentService, ProviderRegistry};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CreateCouponInput, CreatePromotionInput, PromotionService, PromotionStatus, PromotionTarget,
    PromotionType, UpdatePromotionInput,
};
use fastcart_module_shipping::{
    CreateShippingOptionInput, CreateShippingProfileInput, ShippingService,
};
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

fn usd() -> CurrencyCode {
    CurrencyCode::new("USD").unwrap()
}

/// A store with one 2000-cent variant and a 500-cent shipping option, plus the
/// workflow stack every test here starts from.
struct Store {
    db: StoreDatabase,
    variant_id: String,
    shipping_option_id: fastcart_kernel::ShippingOptionId,
    context: CartContext,
    carts: CartWorkflow,
}

impl Store {
    async fn new() -> Self {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let currency = usd();
        let catalog = CatalogService::new(db.clone());
        let product = catalog
            .create_product(CreateProductInput {
                title: "Order deal".to_owned(),
                variants: vec![CreateVariantInput {
                    title: "Default".to_owned(),
                    sku: Some("ORDER-1".to_owned()),
                    manage_inventory: Some(false),
                    ..Default::default()
                }],
                ..Default::default()
            })
            .await
            .unwrap();
        catalog.publish_product(&product.id).await.unwrap();
        let variant_id = product.variants[0].id.clone();
        PricingService::new(db.clone())
            .create_price(CreatePriceInput::new(&variant_id, currency.clone(), 2_000))
            .await
            .unwrap();

        let shipping = ShippingService::new(db.clone());
        let profile = shipping
            .create_profile(CreateShippingProfileInput {
                name: "Default".to_owned(),
                profile_type: None,
                metadata: None,
            })
            .await
            .unwrap();
        let option = shipping
            .create_option(CreateShippingOptionInput {
                profile_id: profile.id.clone(),
                name: "Standard".to_owned(),
                amount_minor: 500,
                currency: currency.clone(),
                provider_id: None,
                provider_data: None,
                fulfillment_set_id: None,
                is_enabled: Some(true),
                metadata: None,
            })
            .await
            .unwrap();

        Self {
            db: db.clone(),
            variant_id,
            shipping_option_id: option.id,
            context: CartContext::new(CountryCode::new("US").unwrap()),
            carts: CartWorkflow::new(db),
        }
    }

    /// A cart holding two units of the variant with shipping selected:
    /// subtotal 4000, shipping 500.
    async fn cart(&self) -> fastcart_module_cart::Cart {
        let carts = &self.carts;
        let cart = carts
            .create_cart(CreateCartInput {
                currency_code: usd().to_string(),
                customer_id: None,
                email: Some("buyer@example.com".to_owned()),
                sales_channel_id: None,
                region_id: None,
                metadata: Metadata::empty(),
                expires_in_seconds: None,
            })
            .await
            .unwrap();
        let cart = carts
            .add_item(
                &cart.id,
                &VariantId::from_string(self.variant_id.clone()),
                2,
                &self.context,
            )
            .await
            .unwrap();
        carts
            .set_shipping_method(&cart.id, &self.shipping_option_id, &self.context)
            .await
            .unwrap()
    }

    fn promotions(&self) -> PromotionService {
        PromotionService::new(self.db.clone())
    }
}

/// The order-target discount shows up in the cart totals and is the amount the
/// customer pays: the order total must equal the cart total it was placed from.
#[tokio::test]
async fn an_order_promotion_discounts_the_cart_and_the_order_that_places_it() {
    let store = Store::new().await;
    store
        .promotions()
        .create_promotion(
            CreatePromotionInput::new("Ten percent of the order", PromotionType::Percentage, 1_000)
                .active()
                .automatic()
                .with_target(PromotionTarget::Order),
        )
        .await
        .unwrap();

    let cart = store.cart().await;

    // 10% of (4000 + 500) = 450, one cart-level discount.
    assert_eq!(cart.totals.item_subtotal, 4_000);
    assert_eq!(cart.totals.shipping_total, 500);
    assert_eq!(cart.totals.discount_total, 450);
    assert_eq!(cart.totals.total, 4_050);
    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.discounts[0].amount, 450);

    // Recalculating the same cart does not accumulate a second row: the
    // discount is re-derived, not stacked on top of the stored one.
    let again = store
        .carts
        .recalculate(&cart.id, &store.context)
        .await
        .unwrap();
    assert_eq!(again.discounts.len(), 1);
    assert_eq!(again.discounts[0].amount, 450);
    assert_eq!(again.totals.discount_total, 450);
    let cart = again;

    let providers = ProviderRegistry::new();
    providers
        .register(Arc::new(ManualProvider::sandbox()))
        .unwrap();
    let payments = PaymentService::with_registry(store.db.clone(), providers);
    let checkout_workflow = CheckoutWorkflow::new(store.db.clone(), payments);
    let checkout = checkout_workflow
        .create(CreateCheckoutRequest::new(
            cart.id.clone(),
            "manual",
            store.context.clone(),
        ))
        .await
        .unwrap();
    let completed = checkout_workflow
        .complete(&checkout.id, &store.context)
        .await
        .unwrap();

    assert_eq!(
        completed.order.total, cart.totals.total,
        "the order total must equal the cart total it was placed from"
    );
    assert_eq!(completed.order.total, 4_050);
    assert_eq!(completed.order.discount_total, 450);
    assert_eq!(
        completed.payment.amount, cart.totals.total,
        "the customer pays the discounted cart total"
    );
    // Checkout attributes cart-level discounts to lines, so the order's own
    // figures add up to the cart's.
    assert_eq!(
        completed
            .order
            .items
            .iter()
            .map(|item| item.discount_total)
            .sum::<i64>(),
        450
    );
}

/// The default target still discounts the line items and writes no cart-level
/// discount at all: `items` must not pick up the new write path.
#[tokio::test]
async fn an_items_promotion_writes_no_cart_level_discount() {
    let store = Store::new().await;
    store
        .promotions()
        .create_promotion(
            CreatePromotionInput::new("Ten percent", PromotionType::Percentage, 1_000)
                .active()
                .automatic(),
        )
        .await
        .unwrap();

    let cart = store.cart().await;

    assert_eq!(cart.totals.discount_total, 400);
    assert_eq!(cart.totals.total, 4_100);
    assert!(
        cart.discounts.is_empty(),
        "an items promotion keeps its discount on the lines: {:?}",
        cart.discounts
    );
}

/// A cart-level discount is re-derived on every recalculation, so retiring the
/// promotion takes its row off the cart again.
#[tokio::test]
async fn a_retired_order_promotion_loses_its_cart_level_discount() {
    let store = Store::new().await;
    let promotions = store.promotions();
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Ten percent of the order", PromotionType::Percentage, 1_000)
                .active()
                .automatic()
                .with_target(PromotionTarget::Order),
        )
        .await
        .unwrap();

    let cart = store.cart().await;
    assert_eq!(cart.totals.discount_total, 450);
    assert_eq!(cart.discounts.len(), 1);

    promotions
        .update_promotion(
            &promotion.id,
            UpdatePromotionInput {
                status: Some(PromotionStatus::Disabled),
                ..UpdatePromotionInput::default()
            },
        )
        .await
        .unwrap();
    let cart = store
        .carts
        .recalculate(&cart.id, &store.context)
        .await
        .unwrap();

    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 4_500);
    assert!(
        cart.discounts.is_empty(),
        "the retired promotion's row must be gone: {:?}",
        cart.discounts
    );
}

/// A shipping-target promotion discounts the shipping methods, not the items.
#[tokio::test]
async fn a_shipping_promotion_discounts_shipping_only() {
    let store = Store::new().await;
    store
        .promotions()
        .create_promotion(
            CreatePromotionInput::new("Half off shipping", PromotionType::Percentage, 5_000)
                .active()
                .automatic()
                .with_target(PromotionTarget::Shipping),
        )
        .await
        .unwrap();

    let cart = store.cart().await;

    assert_eq!(cart.totals.shipping_discount_total, 250);
    assert_eq!(cart.totals.shipping_total, 250);
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 4_250);
}

/// Replacing the cart-level discounts must not evict the coupon rows the cart
/// carries, and the code must still be there after the recalculations an
/// order-target promotion triggers.
#[tokio::test]
async fn an_order_promotion_keeps_the_applied_coupon_on_the_cart() {
    let store = Store::new().await;
    let promotions = store.promotions();
    let order_promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Ten percent of the order", PromotionType::Percentage, 1_000)
                .active()
                .automatic()
                .with_target(PromotionTarget::Order),
        )
        .await
        .unwrap();
    let coupon_promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Five off", PromotionType::FixedAmount, 500)
                .active()
                .with_code("SAVE500"),
        )
        .await
        .unwrap();
    let coupon = promotions
        .create_coupon(CreateCouponInput::new("SAVE500", &coupon_promotion.id))
        .await
        .unwrap();

    let cart = store.cart().await;
    assert_eq!(
        cart.discounts.len(),
        1,
        "the automatic order promotion already wrote its discount"
    );

    let cart = store
        .carts
        .apply_coupon(&cart.id, &coupon.code, &store.context)
        .await
        .unwrap();

    let coupon_row = cart
        .discounts
        .iter()
        .find(|discount| discount.code.as_deref() == Some("SAVE500"))
        .unwrap_or_else(|| {
            panic!(
                "the coupon must stay on the cart after the order promotion recalculated: {:?}",
                cart.discounts
            )
        });
    assert_eq!(
        coupon_row.promotion_id.as_ref().map(ToString::to_string),
        Some(coupon_promotion.id.clone())
    );
    assert_eq!(
        coupon_row.amount, 0,
        "the coupon row is a placeholder; its discount lives on the lines"
    );

    let order_row = cart
        .discounts
        .iter()
        .find(|discount| {
            discount.promotion_id.as_ref().map(ToString::to_string)
                == Some(order_promotion.id.clone())
        })
        .expect("the order promotion keeps its cart-level discount");
    assert_eq!(order_row.amount, 450);

    // 500 off the lines (from the coupon) and 450 off the whole order.
    assert_eq!(cart.totals.discount_total, 950);
    assert_eq!(cart.totals.total, 3_550);

    // The rows above are the cart as stored, not an in-memory artefact.
    let stored = store.carts.get_cart(&cart.id).await.unwrap();
    assert_eq!(stored.discounts.len(), 2);
    assert!(
        stored
            .discounts
            .iter()
            .any(|discount| discount.code.as_deref() == Some("SAVE500")),
        "the coupon must survive the recalculation in the stored rows: {:?}",
        stored.discounts
    );
}

/// A coupon that unlocks an order-target promotion: the marker row the coupon
/// wrote carries the discount, and removing the coupon takes it away again.
#[tokio::test]
async fn a_coupon_for_an_order_promotion_discounts_through_its_own_row() {
    let store = Store::new().await;
    let promotions = store.promotions();
    let promotion = promotions
        .create_promotion(
            CreatePromotionInput::new("Ten percent of the order", PromotionType::Percentage, 1_000)
                .active()
                .with_code("ORDER10")
                .with_target(PromotionTarget::Order),
        )
        .await
        .unwrap();
    promotions
        .create_coupon(CreateCouponInput::new("ORDER10", &promotion.id))
        .await
        .unwrap();

    let cart = store.cart().await;
    assert!(
        cart.discounts.is_empty(),
        "an inactive coupon discounts nothing: {:?}",
        cart.discounts
    );
    assert_eq!(cart.totals.total, 4_500);

    let cart = store
        .carts
        .apply_coupon(&cart.id, "ORDER10", &store.context)
        .await
        .unwrap();
    assert_eq!(cart.totals.discount_total, 450);
    assert_eq!(cart.totals.total, 4_050);
    assert_eq!(
        cart.discounts.len(),
        1,
        "the discount belongs on the coupon's own row, not a second one: {:?}",
        cart.discounts
    );
    assert_eq!(cart.discounts[0].code.as_deref(), Some("ORDER10"));
    assert_eq!(cart.discounts[0].amount, 450);
    assert!(
        !cart.discounts[0].is_automatic,
        "the row stays the coupon's own, applied by the customer"
    );

    let cart = store
        .carts
        .remove_coupon(&cart.id, "ORDER10", &store.context)
        .await
        .unwrap();
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 4_500);
    assert!(cart.discounts.is_empty(), "{:?}", cart.discounts);
}
