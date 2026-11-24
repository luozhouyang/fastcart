use super::*;

#[tokio::test]
async fn calculates_totals_from_stored_snapshots() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 2))
        .await
        .unwrap();
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 500, 1))
        .await
        .unwrap();

    assert_eq!(cart.totals.item_subtotal, 2_500);
    assert_eq!(cart.totals.subtotal, 2_500);
    assert_eq!(cart.totals.discount_total, 0);
    assert_eq!(cart.totals.total, 2_500);
}

#[tokio::test]
async fn caps_line_adjustments_at_the_line_subtotal() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .add_item_adjustment(&cart.id, &item_id, adjustment(1_500))
        .await
        .unwrap();
    assert_eq!(cart.totals.item_discount_total, 1_000);
    assert_eq!(cart.totals.discount_total, 1_000);
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn caps_cart_discounts_at_the_item_subtotal() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();

    let cart = service
        .add_discount(
            &cart.id,
            DiscountInput {
                promotion_id: Some(PromotionId::new()),
                code: Some("SAVE".to_owned()),
                amount: 1_500,
                is_automatic: false,
                metadata: Metadata::empty(),
            },
        )
        .await
        .unwrap();

    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.totals.discount_total, 1_000);
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn inclusive_tax_is_informational_and_not_added_to_the_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .replace_tax_lines(&cart.id, vec![tax_line(100, true)])
        .await
        .unwrap();

    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.totals.tax_total, 100);
    assert_eq!(cart.totals.total, 1_000);
}

#[tokio::test]
async fn exclusive_tax_is_added_to_the_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .replace_tax_lines(&cart.id, vec![tax_line(100, false), tax_line(50, true)])
        .await
        .unwrap();

    assert_eq!(cart.totals.tax_total, 150);
    assert_eq!(cart.totals.total, 1_100);
}
