//! Boundary tests for the cart's commercial calculator.
//!
//! [`calculate_totals`] derives every figure from stored snapshots and promises
//! to saturate instead of overflowing. These tests pin that promise down at the
//! edges of the `i64` range: no panic, no wrap-around, and no negative total.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{Metadata, ProductId, PromotionId, ShippingOptionId, StoreId, VariantId};
use fastcart_module_cart::{
    AddItemInput, Cart, CartAdjustment, CartService, CartShippingMethod, CartTaxLine,
    CreateCartInput, DiscountInput, MAX_ITEM_QUANTITY, calculate_totals,
};

const MAX: i64 = i64::MAX;

fn line(unit_price: i64, quantity: i64) -> AddItemInput {
    AddItemInput {
        product_id: ProductId::new(),
        variant_id: VariantId::new(),
        title: "Line".to_owned(),
        variant_title: None,
        sku: None,
        thumbnail: None,
        unit_price,
        currency_code: "USD".to_owned(),
        quantity,
        is_tax_inclusive: false,
        requires_shipping: true,
        metadata: Metadata::empty(),
    }
}

fn adjustment(amount: i64) -> CartAdjustment {
    CartAdjustment {
        promotion_id: None,
        code: None,
        amount,
        description: None,
    }
}

fn shipping(amount: i64) -> CartShippingMethod {
    CartShippingMethod {
        shipping_option_id: ShippingOptionId::new(),
        name: "Express".to_owned(),
        amount,
        currency: fastcart_kernel::CurrencyCode::new("USD").unwrap(),
        is_tax_inclusive: false,
        data: serde_json::json!({}),
    }
}

fn tax_line(amount: i64, is_tax_inclusive: bool) -> CartTaxLine {
    CartTaxLine {
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive,
        cart_item_id: None,
        shipping_method_id: None,
    }
}

async fn service() -> (StoreDatabase, CartService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CartService::new(db.clone());
    (db, service)
}

/// Builds a cart whose lines are added through the service, optionally with a
/// cart-level discount, and returns the stored aggregate.
async fn cart_with(service: &CartService, lines: Vec<AddItemInput>, discount: i64) -> Cart {
    let cart = service
        .create_cart(CreateCartInput::new("USD"))
        .await
        .unwrap();
    let mut cart = cart;
    for line in lines {
        cart = service.add_item(&cart.id, line).await.unwrap();
    }
    if discount <= 0 {
        return cart;
    }
    service
        .add_discount(
            &cart.id,
            DiscountInput {
                promotion_id: Some(PromotionId::new()),
                code: Some("SAVE".to_owned()),
                amount: discount,
                is_automatic: false,
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_line_at_the_price_limit_saturates_its_subtotal() {
    let (_, service) = service().await;
    let cart = cart_with(&service, vec![line(MAX, MAX_ITEM_QUANTITY)], 0).await;

    // Plain multiplication would panic in a debug build and wrap to a negative
    // line subtotal in a release build.
    assert_eq!(MAX.wrapping_mul(2), -2);
    assert_eq!(cart.totals.item_subtotal, MAX);
    assert_eq!(cart.totals.subtotal, MAX);
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, MAX);
    assert!(cart.totals.total >= 0, "{:?}", cart.totals);
}

#[tokio::test]
async fn several_lines_near_the_limit_saturate_the_item_subtotal() {
    let (_, service) = service().await;
    let half = MAX / 2 + 1;
    let cart = cart_with(&service, vec![line(half, 2), line(half, 2), line(1, 1)], 0).await;

    // Each of the first two lines already drains the whole range; the third
    // must not wrap the sum back to a negative value.
    assert_eq!(half.wrapping_mul(2), i64::MIN);
    assert_eq!(cart.totals.item_subtotal, MAX);
    assert_eq!(cart.totals.total, MAX);
    assert!(cart.totals.total >= 0, "{:?}", cart.totals);
}

#[tokio::test]
async fn line_adjustments_at_the_limit_are_capped_by_their_line() {
    let (_, service) = service().await;
    let mut cart = cart_with(&service, vec![line(1_000, 1)], 0).await;
    cart.items[0].adjustments = vec![adjustment(MAX)];

    let totals = calculate_totals(&cart, &cart.tax_lines);
    assert_eq!(totals.item_subtotal, 1_000);
    assert_eq!(totals.item_discount_total, 1_000);
    assert_eq!(totals.discount_total, 1_000);
    assert_eq!(totals.total, 0);
}

#[tokio::test]
async fn a_cart_discount_at_the_limit_is_capped_by_the_item_subtotal() {
    let (_, service) = service().await;
    let cart = cart_with(&service, vec![line(1_000, 1)], MAX).await;

    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.totals.item_subtotal, 1_000);
    assert_eq!(cart.totals.discount_total, 1_000);
    assert_eq!(cart.totals.total, 0);
    assert!(
        cart.totals.item_subtotal - cart.totals.discount_total >= 0,
        "{:?}",
        cart.totals
    );
}

#[tokio::test]
async fn shipping_at_the_limit_saturates_and_discounts_cap_against_it() {
    let (_, service) = service().await;
    let mut cart = cart_with(&service, vec![line(1_000, 1)], 0).await;
    // `set_shipping_method` keeps one method; stored rows are still untrusted,
    // so two extreme methods must saturate instead of wrapping.
    cart.shipping_methods = vec![shipping(MAX), shipping(MAX)];
    cart.shipping_adjustments = vec![adjustment(MAX)];

    let totals = calculate_totals(&cart, &cart.tax_lines);
    assert_eq!(totals.shipping_subtotal, MAX);
    assert_eq!(totals.shipping_discount_total, MAX);
    assert_eq!(totals.shipping_total, 0);
    assert_eq!(totals.total, 1_000);
}

#[tokio::test]
async fn exclusive_tax_at_the_limit_saturates_the_tax_and_the_total() {
    let (_, service) = service().await;
    let mut cart = cart_with(&service, vec![line(1_000, 1)], 0).await;
    cart.tax_lines = vec![tax_line(MAX, false), tax_line(MAX, false)];

    let totals = calculate_totals(&cart, &cart.tax_lines);
    assert_eq!(totals.tax_total, MAX);
    assert_eq!(totals.total, MAX);
}

#[tokio::test]
async fn negative_stored_snapshots_are_clamped_away() {
    let (_, service) = service().await;
    let mut cart = cart_with(&service, vec![line(1_000, 1)], 0).await;
    // A corrupted row (a negative price, quantity, adjustment or discount)
    // must not turn into a positive amount anywhere in the summary.
    cart.items[0].unit_price = -1;
    cart.items[0].adjustments = vec![adjustment(-500)];
    cart.discounts = vec![fastcart_module_cart::CartDiscount {
        promotion_id: None,
        code: None,
        amount: -700,
        is_automatic: false,
        metadata: Metadata::empty(),
    }];

    let totals = calculate_totals(&cart, &cart.tax_lines);
    assert_eq!(totals.item_subtotal, 0);
    assert_eq!(totals.discount_total, 0);
    assert_eq!(totals.total, 0);
}

/// Regression: the headroom of two lines can each reach the top of the
/// `i64` range, so their sum does not fit in it.
///
/// Two lines can each reach the top of the `i64` range, so their headrooms sum
/// beyond it. `allocate_cart_discounts` saturates that sum into `total` and
/// hands `total` to `allocate_proportionally`, whose `debug_assert_eq!` then
/// adds the same two parts again: in a debug build this panics with "attempt
/// to add with overflow", and in a release build the impossible `total` makes
/// the smallest cart-level discount overshoot the amount it must equal.
#[tokio::test]
async fn attribution_across_saturated_lines_never_panics_and_stays_exact() {
    let (_, service) = service().await;
    let cart = cart_with(&service, vec![line(MAX, 1), line(MAX, 1)], 1).await;

    // The parts sum past the top of the range; a saturating sum loses the
    // information the allocation needs.
    assert_eq!(MAX.wrapping_add(MAX), -2);
    assert_eq!(cart.totals.discount_total, 1);

    let shares = fastcart_module_cart::allocate_cart_discounts(&cart).unwrap();
    assert_eq!(shares.iter().sum::<i64>(), cart.totals.discount_total);
    assert!(
        shares
            .iter()
            .zip([MAX, MAX])
            .all(|(share, part)| *share <= part),
        "a line must never be discounted past its own value: {shares:?}"
    );
}
