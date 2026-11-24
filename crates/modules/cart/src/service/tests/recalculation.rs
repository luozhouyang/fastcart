use super::*;

#[tokio::test]
async fn recalculate_returns_current_totals() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 300, 3))
        .await
        .unwrap();

    let totals = service.recalculate(&cart.id).await.unwrap();
    assert_eq!(totals.item_subtotal, 900);
    assert_eq!(totals.currency_code, CurrencyCode::new("USD").unwrap());

    let missing = fastcart_kernel::CartId::new();
    assert!(service.recalculate(&missing).await.is_err());
}
