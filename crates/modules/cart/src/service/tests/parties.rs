use super::*;

#[tokio::test]
async fn stores_shipping_and_billing_addresses_separately() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    let shipping = AddressInput {
        first_name: Some("Ada".to_owned()),
        city: Some("London".to_owned()),
        country_code: Some("gb".to_owned()),
        ..AddressInput::default()
    };
    let billing = AddressInput {
        first_name: Some("Grace".to_owned()),
        city: Some("Arlington".to_owned()),
        country_code: Some("us".to_owned()),
        ..AddressInput::default()
    };

    let cart = service
        .set_shipping_address(&cart.id, shipping)
        .await
        .unwrap();
    let cart = service
        .set_billing_address(&cart.id, billing)
        .await
        .unwrap();

    assert_eq!(cart.shipping_address.first_name.as_deref(), Some("Ada"));
    assert_eq!(cart.shipping_address.country_code.as_deref(), Some("GB"));
    assert_eq!(cart.billing_address.first_name.as_deref(), Some("Grace"));
    assert_eq!(cart.billing_address.country_code.as_deref(), Some("US"));
}

#[tokio::test]
async fn attaching_a_different_customer_conflicts() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let first = CustomerId::new();
    let second = CustomerId::new();

    let cart = service
        .attach_customer(&cart.id, &first, Some("One@Example.com".to_owned()))
        .await
        .unwrap();
    assert_eq!(cart.customer_id.as_ref(), Some(&first));
    assert_eq!(cart.email.as_deref(), Some("one@example.com"));

    let error = service
        .attach_customer(&cart.id, &second, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));
}

#[tokio::test]
async fn attach_customer_never_overwrites_a_concurrent_owner() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;
    let first = CustomerId::new();
    let second = CustomerId::new();

    // Simulate another writer attaching a customer after this request
    // loaded the cart but before it updated it.
    sqlx::query("UPDATE carts SET customer_id = ? WHERE id = ?")
        .bind(first.as_str())
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let error = service
        .attach_customer(&cart.id, &second, None)
        .await
        .unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Conflict(_)));

    let stored = service.get_cart(&cart.id).await.unwrap();
    assert_eq!(stored.customer_id.as_ref(), Some(&first));
}

#[tokio::test]
async fn malformed_stored_address_snapshot_is_a_database_error() {
    let service = service().await;
    let cart = create_cart(&service, "USD").await;

    sqlx::query("UPDATE carts SET shipping_address = ? WHERE id = ?")
        .bind("{not json")
        .bind(cart.id.as_str())
        .execute(service.db.pool())
        .await
        .unwrap();

    let error = service.get_cart(&cart.id).await.unwrap_err();
    assert!(matches!(error, fastcart_kernel::AppError::Database(_)));
}
