//! Coupon flows when the code on the cart came from an automatic promotion.
//!
//! An automatic order-target promotion writes its own code onto the cart-level
//! row it owns, so the code is already on the cart before the customer types it.
//! Entering it is not a repeat and must not be reported as one or stacked twice;
//! removing it is not the customer's to do, because the next recalculation
//! re-derives the row regardless. Codes the customer applied themselves keep the
//! behaviour they always had: re-entering them conflicts and removing them takes
//! the discount away.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CountryCode, CurrencyCode, Metadata, StoreId, VariantId};
use fastcart_module_cart::{Cart, CreateCartInput};
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};
use fastcart_module_pricing::{CreatePriceInput, PricingService};
use fastcart_module_promotion::{
    CreateCouponInput, CreatePromotionInput, PromotionService, PromotionTarget, PromotionType,
};
use fastcart_workflows::{CartContext, CartWorkflow};

const AUTOMATIC_CODE: &str = "AUTO10";
const CUSTOMER_CODE: &str = "SAVE10";

/// A store with one 2000-cent variant and the workflow stack every test starts
/// from.
struct Store {
    db: StoreDatabase,
    workflow: CartWorkflow,
    context: CartContext,
    variant_id: String,
}

impl Store {
    async fn new() -> Self {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let currency = CurrencyCode::new("USD").unwrap();

        let catalog = CatalogService::new(db.clone());
        let product = catalog
            .create_product(CreateProductInput {
                title: "Mug".to_owned(),
                variants: vec![CreateVariantInput {
                    title: "Default".to_owned(),
                    sku: Some("MUG-1".to_owned()),
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
            .create_price(CreatePriceInput::new(&variant_id, currency, 2_000))
            .await
            .unwrap();

        Self {
            db: db.clone(),
            workflow: CartWorkflow::new(db),
            context: CartContext::new(CountryCode::new("US").unwrap()),
            variant_id,
        }
    }

    fn promotions(&self) -> PromotionService {
        PromotionService::new(self.db.clone())
    }

    /// One unit of the variant, recalculated by `add_item`.
    async fn cart(&self) -> Cart {
        let cart = self
            .workflow
            .create_cart(CreateCartInput {
                currency_code: CurrencyCode::new("USD").unwrap().to_string(),
                customer_id: None,
                email: Some("buyer@example.com".to_owned()),
                sales_channel_id: None,
                region_id: None,
                metadata: Metadata::empty(),
                expires_in_seconds: None,
            })
            .await
            .unwrap();
        self.workflow
            .add_item(
                &cart.id,
                &VariantId::from_string(self.variant_id.clone()),
                1,
                &self.context,
            )
            .await
            .unwrap()
    }

    /// An automatic order-target 10 % promotion that also carries a code, plus
    /// the coupon row that lets a customer type that code.
    async fn automatic_promotion_with_code(&self, code: &str) -> String {
        let promotions = self.promotions();
        let promotion = promotions
            .create_promotion(
                CreatePromotionInput::new(
                    "Ten percent of the order",
                    PromotionType::Percentage,
                    1_000,
                )
                .with_code(code)
                .with_target(PromotionTarget::Order)
                .active()
                .automatic(),
            )
            .await
            .unwrap();
        promotions
            .create_coupon(CreateCouponInput::new(code, &promotion.id))
            .await
            .unwrap();
        promotion.id
    }

    /// An order-target 10 % promotion unlocked by the customer entering a code.
    async fn customer_coupon(&self, code: &str) -> String {
        let promotions = self.promotions();
        let promotion = promotions
            .create_promotion(
                CreatePromotionInput::new(
                    "Ten percent of the order",
                    PromotionType::Percentage,
                    1_000,
                )
                .with_code(code)
                .with_target(PromotionTarget::Order)
                .active(),
            )
            .await
            .unwrap();
        promotions
            .create_coupon(CreateCouponInput::new(code, &promotion.id))
            .await
            .unwrap();
        promotion.id
    }
}

/// Entering the code of the automatic promotion that already discounted the
/// cart succeeds and leaves exactly the one discount where it was.
#[tokio::test]
async fn applying_the_code_of_an_automatic_promotion_is_idempotent() {
    let store = Store::new().await;
    let promotion_id = store.automatic_promotion_with_code(AUTOMATIC_CODE).await;
    let cart = store.cart().await;

    // The automatic pass already wrote its code onto the cart-level row.
    assert_eq!(cart.discounts.len(), 1);
    let automatic = &cart.discounts[0];
    assert_eq!(
        automatic.promotion_id.as_ref().map(ToString::to_string),
        Some(promotion_id)
    );
    assert_eq!(automatic.code.as_deref(), Some(AUTOMATIC_CODE));
    assert_eq!(automatic.amount, 200);
    assert!(automatic.is_automatic);

    let cart = store
        .workflow
        .apply_coupon(&cart.id, AUTOMATIC_CODE, &store.context)
        .await
        .unwrap();

    assert_eq!(
        cart.discounts.len(),
        1,
        "the code must not stack a second discount: {:?}",
        cart.discounts
    );
    assert!(
        cart.discounts[0].is_automatic,
        "the row stays the promotion's own: {:?}",
        cart.discounts
    );
    assert_eq!(cart.discounts[0].amount, 200);
    assert_eq!(cart.totals.discount_total, 200);
    assert_eq!(cart.totals.total, 1_800);

    let stored = store.workflow.get_cart(&cart.id).await.unwrap();
    assert_eq!(stored.discounts.len(), 1);
    assert_eq!(stored.totals.discount_total, 200);
}

/// The control: re-entering a code the customer applied themselves is still the
/// conflict it always was.
#[tokio::test]
async fn reapplying_a_customer_coupon_still_conflicts() {
    let store = Store::new().await;
    store.customer_coupon(CUSTOMER_CODE).await;
    let cart = store.cart().await;

    let cart = store
        .workflow
        .apply_coupon(&cart.id, CUSTOMER_CODE, &store.context)
        .await
        .unwrap();
    assert_eq!(cart.discounts.len(), 1);
    assert!(!cart.discounts[0].is_automatic);
    assert_eq!(cart.totals.discount_total, 200);

    let error = store
        .workflow
        .apply_coupon(&cart.id, CUSTOMER_CODE, &store.context)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AppError::Conflict(message) if message.contains("already applied")),
        "a repeated customer code keeps its conflict: {error:?}"
    );
    assert_eq!(cart.totals.discount_total, 200);
}

/// Removing a code an automatic promotion carries is refused: nothing of the
/// customer's would be removed, the row is re-derived by every recalculation,
/// and reporting success would claim a change that cannot hold.
#[tokio::test]
async fn removing_an_automatic_code_is_refused_and_keeps_the_discount() {
    let store = Store::new().await;
    store.automatic_promotion_with_code(AUTOMATIC_CODE).await;
    let cart = store.cart().await;
    assert_eq!(cart.totals.discount_total, 200);

    let error = store
        .workflow
        .remove_coupon(&cart.id, AUTOMATIC_CODE, &store.context)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AppError::Conflict(message)
            if message.contains(AUTOMATIC_CODE) && message.contains("automatic")),
        "the refusal must name the code and say why: {error:?}"
    );

    let stored = store.workflow.get_cart(&cart.id).await.unwrap();
    assert_eq!(stored.discounts.len(), 1);
    assert_eq!(stored.discounts[0].code.as_deref(), Some(AUTOMATIC_CODE));
    assert_eq!(stored.totals.discount_total, 200);
    assert_eq!(stored.totals.total, 1_800);

    // The refusal is not the row surviving a moment: the discount is still
    // there on the recalculation that follows.
    let cart = store
        .workflow
        .recalculate(&cart.id, &store.context)
        .await
        .unwrap();
    assert_eq!(cart.totals.discount_total, 200);
    assert_eq!(cart.totals.total, 1_800);
}

/// The control: a code the customer applied is removed exactly as before.
#[tokio::test]
async fn removing_a_customer_coupon_still_takes_the_discount_away() {
    let store = Store::new().await;
    store.customer_coupon(CUSTOMER_CODE).await;
    let cart = store.cart().await;

    let cart = store
        .workflow
        .apply_coupon(&cart.id, CUSTOMER_CODE, &store.context)
        .await
        .unwrap();
    assert_eq!(cart.totals.discount_total, 200);

    let cart = store
        .workflow
        .remove_coupon(&cart.id, CUSTOMER_CODE, &store.context)
        .await
        .unwrap();
    assert!(cart.discounts.is_empty(), "{:?}", cart.discounts);
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 2_000);
}
