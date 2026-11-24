use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{Metadata, ProductId, PromotionId, StoreId, VariantId};

use crate::domain::{AddItemInput, CreateCartInput, DiscountInput, allocate_cart_discounts};
use crate::service::CartService;

fn item(unit_price: i64, quantity: i64) -> AddItemInput {
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

async fn cart_with(lines: Vec<AddItemInput>, discount: i64) -> crate::domain::Cart {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CartService::new(db);
    let cart = service
        .create_cart(CreateCartInput::new("USD"))
        .await
        .unwrap();
    for line in lines {
        service.add_item(&cart.id, line).await.unwrap();
    }
    if discount > 0 {
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
    } else {
        service.get_cart(&cart.id).await.unwrap()
    }
}

/// Line items only carry their own adjustments, so the cart-level part of
/// `discount_total` has to be attributed to lines before an order can be
/// built from them. Without this the order silently records a total that
/// nobody was charged.
#[tokio::test]
async fn cart_level_discounts_are_attributed_across_items() {
    let cart = cart_with(vec![item(1_000, 1), item(400, 1)], 300).await;

    let shares = allocate_cart_discounts(&cart).unwrap();

    assert_eq!(shares.len(), 2);
    assert_eq!(shares.iter().sum::<i64>(), 300);
    assert!(
        shares[0] > shares[1],
        "the larger line takes more: {shares:?}"
    );
    assert!(shares[0] <= 1_000 && shares[1] <= 400);
    assert_eq!(cart.totals.discount_total, 300);
}

#[tokio::test]
async fn attributed_discounts_never_exceed_the_cart_discount_total() {
    // The discount is larger than the cart, so the totals cap it. The
    // attribution must cap the same way or the order would exceed the
    // amount that was charged.
    let cart = cart_with(vec![item(100, 1), item(50, 1)], 500).await;

    let shares = allocate_cart_discounts(&cart).unwrap();

    assert_eq!(cart.totals.discount_total, 150);
    assert_eq!(shares.iter().sum::<i64>(), cart.totals.discount_total);
    assert!(shares[0] <= 100 && shares[1] <= 50);
}

#[tokio::test]
async fn attribution_is_zero_without_cart_level_discounts() {
    let cart = cart_with(vec![item(100, 1)], 0).await;
    assert_eq!(allocate_cart_discounts(&cart).unwrap(), vec![0]);
}
