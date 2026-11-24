use super::*;

#[tokio::test]
async fn setting_a_shipping_method_replaces_the_previous_selection() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 500))
        .await
        .unwrap();
    assert_eq!(cart.shipping_methods.len(), 1);
    assert_eq!(cart.totals.shipping_subtotal, 500);
    assert_eq!(cart.totals.shipping_total, 500);
    assert_eq!(cart.totals.total, 1_500);

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 800))
        .await
        .unwrap();
    assert_eq!(cart.shipping_methods.len(), 1);
    assert_eq!(cart.shipping_methods[0].amount, 800);
    assert_eq!(cart.totals.total, 1_800);
}

#[tokio::test]
async fn shipping_adjustments_reduce_the_shipping_total() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 500))
        .await
        .unwrap();

    let cart = service
        .replace_shipping_adjustments(&cart.id, vec![adjustment(200)])
        .await
        .unwrap();
    assert_eq!(cart.shipping_adjustments.len(), 1);
    assert_eq!(cart.totals.shipping_discount_total, 200);
    assert_eq!(cart.totals.shipping_total, 300);
    assert_eq!(cart.totals.total, 1_300);
}

#[tokio::test]
async fn changing_shipping_method_clears_stale_shipping_tax_lines() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let first_shipping = shipping_input("USD", 500);
    let first_option = first_shipping.shipping_option_id.clone();
    let cart = service
        .set_shipping_method(&cart.id, first_shipping)
        .await
        .unwrap();

    let cart = service
        .replace_tax_lines(
            &cart.id,
            vec![
                TaxLineInput {
                    name: "Item tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 100,
                    is_tax_inclusive: false,
                    cart_item_id: Some(item_id),
                    shipping_method_id: None,
                },
                TaxLineInput {
                    name: "Shipping tax".to_owned(),
                    rate_bps: 1_000,
                    amount: 50,
                    is_tax_inclusive: false,
                    cart_item_id: None,
                    shipping_method_id: Some(first_option.to_string()),
                },
            ],
        )
        .await
        .unwrap();
    assert_eq!(cart.tax_lines.len(), 2);

    let cart = service
        .set_shipping_method(&cart.id, shipping_input("USD", 700))
        .await
        .unwrap();
    assert_eq!(cart.tax_lines.len(), 1);
    assert_eq!(cart.tax_lines[0].name, "Item tax");
    assert!(
        cart.tax_lines
            .iter()
            .all(|line| line.shipping_method_id.is_none())
    );
}
