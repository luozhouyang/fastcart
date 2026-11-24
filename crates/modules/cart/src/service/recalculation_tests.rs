use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{LineItemId, Metadata, ProductId, PromotionId, StoreId, VariantId};

use crate::domain::{
    AddItemInput, AdjustmentInput, CreateCartInput, RecalculationInput, TaxLineInput,
};
use crate::service::CartService;

async fn cart_with_one_line() -> (CartService, crate::domain::Cart) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CartService::new(db);
    let cart = service
        .create_cart(CreateCartInput::new("USD"))
        .await
        .unwrap();
    let cart = service
        .add_item(
            &cart.id,
            AddItemInput {
                product_id: ProductId::new(),
                variant_id: VariantId::new(),
                title: "Line".to_owned(),
                variant_title: None,
                sku: None,
                thumbnail: None,
                unit_price: 1_000,
                currency_code: "USD".to_owned(),
                quantity: 1,
                is_tax_inclusive: false,
                requires_shipping: true,
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();
    (service, cart)
}

fn adjustment(amount: i64) -> AdjustmentInput {
    AdjustmentInput {
        promotion_id: Some(PromotionId::new()),
        code: Some("PROMO".to_owned()),
        amount,
        description: None,
    }
}

#[tokio::test]
async fn a_recalculation_that_fails_leaves_the_cart_untouched() {
    let (service, cart) = cart_with_one_line().await;
    let item_id = cart.items[0].id.clone();
    let cart = service
        .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(150)])
        .await
        .unwrap();
    assert_eq!(cart.totals.item_discount_total, 150);

    // A tax line naming a line item that belongs to no cart fails
    // validation, which is how a partial write used to happen.
    let foreign = LineItemId::new();
    let error = service
        .apply_recalculation(
            &cart.id,
            RecalculationInput {
                tax_inclusive: None,
                currency_code: "USD".to_owned(),
                prices: vec![],
                item_adjustments: vec![(item_id.clone(), vec![])],
                shipping_adjustments: vec![],
                tax_lines: vec![TaxLineInput {
                    name: "Tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 100,
                    is_tax_inclusive: false,
                    cart_item_id: Some(foreign),
                    shipping_method_id: None,
                }],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let stored = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(
        stored.items[0].adjustments.len(),
        1,
        "the discount must survive a failed recalculation"
    );
    assert_eq!(stored.totals.item_discount_total, 150);
    assert_eq!(stored.items[0].unit_price, 1_000);
    assert!(stored.tax_lines.is_empty());
}

#[tokio::test]
async fn a_recalculation_applies_every_snapshot_at_once() {
    let (service, cart) = cart_with_one_line().await;
    let item_id = cart.items[0].id.clone();

    let cart = service
        .apply_recalculation(
            &cart.id,
            RecalculationInput {
                tax_inclusive: None,
                currency_code: "USD".to_owned(),
                prices: vec![crate::domain::ItemPriceInput {
                    item_id: item_id.clone(),
                    unit_price: 750,
                }],
                item_adjustments: vec![(item_id.clone(), vec![adjustment(100)])],
                shipping_adjustments: vec![],
                tax_lines: vec![TaxLineInput {
                    name: "Tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 65,
                    is_tax_inclusive: false,
                    cart_item_id: Some(item_id),
                    shipping_method_id: None,
                }],
            },
        )
        .await
        .unwrap();

    assert_eq!(cart.items[0].unit_price, 750);
    assert_eq!(cart.items[0].adjustments.len(), 1);
    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.totals.item_discount_total, 100);
    assert_eq!(cart.totals.tax_total, 65);
    assert_eq!(cart.totals.total, 715);
}

#[tokio::test]
async fn a_recalculation_rejects_prices_in_another_currency() {
    let (service, cart) = cart_with_one_line().await;
    let error = service
        .apply_recalculation(
            &cart.id,
            RecalculationInput {
                tax_inclusive: None,
                currency_code: "EUR".to_owned(),
                prices: vec![crate::domain::ItemPriceInput {
                    item_id: cart.items[0].id.clone(),
                    unit_price: 10,
                }],
                item_adjustments: vec![],
                shipping_adjustments: vec![],
                tax_lines: vec![],
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
