use super::*;

#[tokio::test]
async fn creates_and_gets_a_cart() {
    let service = service().await;
    let cart = create_cart(&service, "usd").await;

    assert_eq!(cart.currency_code.as_str(), "USD");
    assert_eq!(cart.status, CartStatus::Open);
    assert!(cart.region_id.is_none());
    assert!(cart.items.is_empty());
    assert_eq!(cart.totals.total, 0);
    assert!(cart.expires_at.is_some());

    let loaded = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(loaded.id, cart.id);
}

#[tokio::test]
async fn persists_the_resolved_region_id() {
    let service = service().await;
    let cart = service
        .create_cart(CreateCartInput {
            region_id: Some("reg_test".to_owned()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    assert_eq!(cart.region_id.as_deref(), Some("reg_test"));
    let loaded = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(loaded.region_id.as_deref(), Some("reg_test"));

    // Carts without a region keep a NULL column and load as `None`.
    let legacy = create_cart(&service, "USD").await;
    let stored: Option<String> = sqlx::query_scalar("SELECT region_id FROM carts WHERE id = ?")
        .bind(legacy.id.as_str())
        .fetch_one(service.db.pool())
        .await
        .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
async fn completes_and_expires_carts_and_rejects_mutations() {
    let service = service().await;
    let open = create_cart(&service, "USD").await;
    let completed = service.complete_cart(&open.id).await.unwrap();
    assert_eq!(completed.status, CartStatus::Completed);
    assert!(completed.completed_at.is_some());

    let error = service
        .add_item(&open.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let expiring = service
        .create_cart(CreateCartInput {
            expires_in_seconds: Some(10),
            ..create_input("USD")
        })
        .await
        .unwrap();
    let count = service.expire_due(i64::MAX).await.unwrap();
    assert_eq!(count, 1);

    let expired = service.get_cart(&expiring.id).await.unwrap();
    assert_eq!(expired.status, CartStatus::Expired);
    let error = service
        .add_item(&expiring.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn lists_carts_for_a_customer() {
    let service = service().await;
    let customer = CustomerId::new();
    let other = CustomerId::new();

    let first = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();
    service
        .add_item(&first.id, item_input("USD", variant(), 250, 2))
        .await
        .unwrap();
    service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("EUR")
        })
        .await
        .unwrap();
    service
        .create_cart(CreateCartInput {
            customer_id: Some(other),
            ..create_input("USD")
        })
        .await
        .unwrap();

    let page = service
        .list_carts(Some(&customer), Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 2);
    let with_items = page
        .data
        .iter()
        .find(|cart| !cart.items.is_empty())
        .unwrap();
    assert_eq!(with_items.totals.item_subtotal, 500);
}

#[tokio::test]
async fn finds_newest_active_cart_for_customer() {
    let service = service().await;
    let customer = CustomerId::new();
    let other = CustomerId::new();

    let first = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();
    let second = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("EUR")
        })
        .await
        .unwrap();
    let other_cart = service
        .create_cart(CreateCartInput {
            customer_id: Some(other.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    // Creation timestamps can collide within one clock tick; pin them so
    // the newest-cart ordering is deterministic.
    for (cart_id, created_at) in [
        (&first.id, 1_i64),
        (&second.id, 2_i64),
        (&other_cart.id, 3_i64),
    ] {
        sqlx::query("UPDATE carts SET created_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(cart_id.as_str())
            .execute(service.db.pool())
            .await
            .unwrap();
    }

    let active = service
        .active_cart_for_customer(&customer)
        .await
        .unwrap()
        .expect("customer has an active cart");
    assert_eq!(active.id, second.id);

    // Completing the newest cart falls back to the older open one.
    service.complete_cart(&second.id).await.unwrap();
    let active = service
        .active_cart_for_customer(&customer)
        .await
        .unwrap()
        .expect("older cart is still open");
    assert_eq!(active.id, first.id);

    // Another customer's carts are never returned.
    let active_other = service
        .active_cart_for_customer(&other)
        .await
        .unwrap()
        .expect("other customer has an active cart");
    assert_eq!(active_other.id, other_cart.id);

    // With no open carts the result is None.
    service.complete_cart(&first.id).await.unwrap();
    assert!(
        service
            .active_cart_for_customer(&customer)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .active_cart_for_customer(&CustomerId::new())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_expired_but_unswept_cart_is_not_active() {
    let service = service().await;
    let customer = CustomerId::new();
    let cart = service
        .create_cart(CreateCartInput {
            customer_id: Some(customer.clone()),
            ..create_input("USD")
        })
        .await
        .unwrap();

    sqlx::query("UPDATE carts SET expires_at = ? WHERE id = ?")
        .bind(now_timestamp() - 1)
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    assert!(
        service
            .active_cart_for_customer(&customer)
            .await
            .unwrap()
            .is_none()
    );

    let error = service
        .add_item(&cart.id, item_input("USD", variant(), 100, 1))
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}
