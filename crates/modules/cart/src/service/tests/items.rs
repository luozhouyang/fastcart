use super::*;

#[tokio::test]
async fn adding_the_same_variant_increments_quantity() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let variant = variant();

    let cart = service
        .add_item(&cart.id, item_input("USD", variant.clone(), 1_000, 2))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    assert_eq!(cart.items[0].quantity, 2);

    let cart = service
        .add_item(&cart.id, item_input("USD", variant, 1_000, 3))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    assert_eq!(cart.items[0].quantity, 5);
    assert_eq!(cart.totals.item_subtotal, 5_000);
}

#[tokio::test]
async fn updates_removes_and_clears_items() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 500, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 4 })
        .await
        .unwrap();
    assert_eq!(cart.items[0].quantity, 4);
    assert_eq!(cart.totals.item_subtotal, 2_000);

    let cart = service
        .update_item_quantity(&cart.id, &item_id, UpdateItemInput { quantity: 0 })
        .await
        .unwrap();
    assert!(cart.items.is_empty());

    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap();
    assert_eq!(cart.items.len(), 1);
    let cart = service.clear_items(&cart.id).await.unwrap();
    assert!(cart.items.is_empty());
    assert_eq!(cart.totals.total, 0);
}

#[tokio::test]
async fn rejects_a_mismatched_item_currency() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let error = service
        .add_item(&cart.id, item_input("EUR", variant(), 1_000, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn validates_quantities_and_unknown_items() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 0))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));

    let missing = LineItemId::new();
    let error = service.remove_item(&cart.id, &missing).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::NotFound(_)));
}

#[tokio::test]
async fn add_item_on_a_completed_cart_is_rejected() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    service.complete_cart(&cart.id).await.unwrap();

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let stored = service.get_cart(&cart.id).await.unwrap();
    assert!(stored.items.is_empty());
}

#[tokio::test]
async fn reprices_items_and_replaces_adjustments() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let cart = service
        .add_item(&cart.id, item_input("USD", variant(), 1_000, 1))
        .await
        .unwrap();
    let item_id = cart.items[0].id.clone();

    let cart = service
        .set_item_unit_price(&cart.id, &item_id, 750, "usd")
        .await
        .unwrap();
    assert_eq!(cart.items[0].unit_price, 750);

    let cart = service
        .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(100)])
        .await
        .unwrap();
    assert_eq!(cart.items[0].adjustments.len(), 1);
    assert_eq!(cart.totals.item_discount_total, 100);

    let cart = service
        .replace_item_adjustments(&cart.id, &item_id, vec![adjustment(50)])
        .await
        .unwrap();
    assert_eq!(cart.items[0].adjustments.len(), 1);
    assert_eq!(cart.totals.item_discount_total, 50);

    let error = service
        .set_item_unit_price(&cart.id, &item_id, 100, "EUR")
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
