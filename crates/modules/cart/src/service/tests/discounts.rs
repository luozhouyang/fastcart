use super::*;

#[tokio::test]
async fn replaces_and_removes_discounts() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let promotion = PromotionId::new();

    let cart = service
        .replace_discounts(
            &cart.id,
            vec![DiscountInput {
                promotion_id: Some(promotion.clone()),
                code: Some("SAVE10".to_owned()),
                amount: 100,
                is_automatic: false,
                metadata: Metadata::empty(),
            }],
        )
        .await
        .unwrap();
    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.totals.total, 900);

    let cart = service.remove_discount(&cart.id, &promotion).await.unwrap();
    assert!(cart.discounts.is_empty());
    assert_eq!(cart.totals.total, 1_000);

    let error = service
        .remove_discount(&cart.id, &promotion)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
}

#[tokio::test]
async fn applying_the_same_discount_twice_updates_one_row() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let promotion = PromotionId::new();
    let discount = |amount| DiscountInput {
        promotion_id: Some(promotion.clone()),
        code: Some("SAVE".to_owned()),
        amount,
        is_automatic: false,
        metadata: Metadata::empty(),
    };

    let cart = service.add_discount(&cart.id, discount(100)).await.unwrap();
    assert_eq!(cart.discounts.len(), 1);

    let cart = service.add_discount(&cart.id, discount(250)).await.unwrap();
    assert_eq!(cart.discounts.len(), 1);
    assert_eq!(cart.discounts[0].amount, 250);
    assert_eq!(cart.totals.discount_total, 250);
}
