use super::*;

#[tokio::test]
async fn update_address_keeps_existing_defaults_when_target_is_missing() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();

    let mut first_input = address("1 Main St");
    first_input.is_default_shipping = true;
    let first = service
        .add_address(&customer.id, first_input)
        .await
        .unwrap();

    let mut replacement = address("2 Main St");
    replacement.is_default_shipping = true;
    let error = service
        .update_address(&customer.id, "addr_missing", replacement)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));

    let unchanged = service.get_address(&customer.id, &first.id).await.unwrap();
    assert!(unchanged.is_default_shipping);
    assert_eq!(unchanged.address_1.as_deref(), Some("1 Main St"));
}

#[tokio::test]
async fn address_crud_and_default_switching() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();

    let mut first_input = address("1 Main St");
    first_input.is_default_shipping = true;
    let first = service
        .add_address(&customer.id, first_input)
        .await
        .unwrap();
    assert!(first.is_default_shipping);
    assert!(!first.is_default_billing);
    assert_eq!(
        first.country_code.as_ref().map(CountryCode::as_str),
        Some("US")
    );

    let mut second_input = address("2 Main St");
    second_input.is_default_shipping = true;
    second_input.is_default_billing = true;
    let second = service
        .add_address(&customer.id, second_input)
        .await
        .unwrap();

    let first_after = service.get_address(&customer.id, &first.id).await.unwrap();
    assert!(!first_after.is_default_shipping);

    let listed = service.list_addresses(&customer.id).await.unwrap();
    assert_eq!(listed.len(), 2);

    let mut update_input = address("3 Main St");
    update_input.is_default_billing = true;
    let updated = service
        .update_address(&customer.id, &first.id, update_input)
        .await
        .unwrap();
    assert_eq!(updated.address_1.as_deref(), Some("3 Main St"));
    assert!(updated.is_default_billing);
    assert!(!updated.is_default_shipping);

    let second_after = service.get_address(&customer.id, &second.id).await.unwrap();
    assert!(!second_after.is_default_billing);

    service
        .remove_address(&customer.id, &first.id)
        .await
        .unwrap();
    let remaining = service.list_addresses(&customer.id).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, second.id);
    assert!(
        service
            .remove_address(&customer.id, &first.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn address_validation_rejects_incomplete_input() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();

    let missing_city = AddressInput::new("1 Main St", "  ", "12345", "US");
    let error = service
        .add_address(&customer.id, missing_city)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let invalid_country = AddressInput::new("1 Main St", "Springfield", "12345", "USA");
    let error = service
        .add_address(&customer.id, invalid_country)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    assert!(
        service
            .add_address(&CustomerId::new(), address("1 Main St"))
            .await
            .is_err()
    );
}
