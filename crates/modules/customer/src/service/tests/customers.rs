use super::*;

#[tokio::test]
async fn creates_and_fetches_a_customer() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(
            CreateCustomerInput::new("  Owner@Example.COM ").with_name("Ada", "Lovelace"),
        )
        .await
        .unwrap();

    assert_eq!(customer.email, "owner@example.com");
    assert_eq!(customer.first_name.as_deref(), Some("Ada"));
    assert_eq!(customer.last_name.as_deref(), Some("Lovelace"));
    assert!(!customer.has_account);
    assert_eq!(customer.status, CustomerStatus::Active);
    assert!(customer.deleted_at.is_none());

    let fetched = service.get_customer(&customer.id).await.unwrap();
    assert_eq!(fetched, customer);

    let by_email = service
        .get_customer_by_email("OWNER@example.com")
        .await
        .unwrap();
    assert_eq!(by_email, Some(customer));
}

#[tokio::test]
async fn missing_customer_is_not_found() {
    let (service, _) = service().await;
    let error = service.get_customer(&CustomerId::new()).await.unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn updates_customer_fields_partially() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com").with_name("Ada", "Lovelace"))
        .await
        .unwrap();

    let updated = service
        .update_customer(
            &customer.id,
            UpdateCustomerInput {
                first_name: Some("Grace".to_owned()),
                status: Some(CustomerStatus::Disabled),
                ..UpdateCustomerInput::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.first_name.as_deref(), Some("Grace"));
    assert_eq!(updated.last_name.as_deref(), Some("Lovelace"));
    assert_eq!(updated.status, CustomerStatus::Disabled);
    assert_eq!(updated.email, customer.email);
}

#[tokio::test]
async fn email_change_updates_credentials_and_clears_verification() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new("owner@example.com", "password"))
        .await
        .unwrap();
    let token = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();
    service.verify_email(&token.token).await.unwrap();

    let updated = service
        .update_customer(
            &customer.id,
            UpdateCustomerInput {
                email: Some("  NewOwner@Example.COM ".to_owned()),
                ..UpdateCustomerInput::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.email, "newowner@example.com");
    assert!(updated.email_verified_at.is_none());

    // Login joins the credential email, which must move with the profile.
    assert!(
        service
            .login("NEWOWNER@example.com", "password")
            .await
            .is_ok()
    );
    assert!(
        service
            .login("owner@example.com", "password")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn update_customer_does_not_fabricate_a_result() {
    let (service, _) = service().await;
    let first = service
        .create_customer(CreateCustomerInput::new("first@example.com"))
        .await
        .unwrap();
    let second = service
        .create_customer(CreateCustomerInput::new("second@example.com"))
        .await
        .unwrap();

    let conflict = service
        .update_customer(
            &first.id,
            UpdateCustomerInput {
                email: Some("SECOND@example.com".to_owned()),
                ..UpdateCustomerInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(conflict, AppError::Conflict(_)));
    assert_eq!(
        service.get_customer(&first.id).await.unwrap().email,
        "first@example.com"
    );

    service.delete_customer(&second.id).await.unwrap();
    let missing = service
        .update_customer(
            &second.id,
            UpdateCustomerInput {
                first_name: Some("Ghost".to_owned()),
                ..UpdateCustomerInput::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)));
}

#[tokio::test]
async fn delete_customer_revokes_sessions_atomically() {
    let (service, db) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new("owner@example.com", "password"))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "password")
        .await
        .unwrap();

    service.delete_customer(&customer.id).await.unwrap();

    let revoked_at: Option<i64> =
        sqlx::query_scalar("SELECT revoked_at FROM customer_sessions WHERE id = ?")
            .bind(session.session_id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    let deleted_at: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM customers WHERE id = ?")
            .bind(customer.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(revoked_at.is_some());
    assert!(deleted_at.is_some());
    assert!(service.authenticate_session(&session.token).await.is_err());
}

#[tokio::test]
async fn guest_creation_is_idempotent_under_concurrency() {
    let (service, db) = service().await;
    let (first, second) = tokio::join!(
        service.find_or_create_guest("guest@example.com", Some("Ada"), None),
        service.find_or_create_guest("guest@example.com", Some("Grace"), None),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.id, second.id);

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers WHERE email = ?")
        .bind("guest@example.com")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn lists_and_searches_customers() {
    let (service, _) = service().await;
    for (email, first) in [
        ("alice@example.com", "Alice"),
        ("bob@example.com", "Bob"),
        ("carol@example.com", "Carol"),
    ] {
        service
            .create_customer(CreateCustomerInput::new(email).with_name(first, "Example"))
            .await
            .unwrap();
    }

    let page = service
        .list_customers(Pagination::new(Some(2), Some(0)).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(page.data.len(), 2);
    assert_eq!(page.count, 3);

    let searched = service
        .list_customers(Pagination::default(), Some("ALI"))
        .await
        .unwrap();
    assert_eq!(searched.count, 1);
    assert_eq!(searched.data[0].email, "alice@example.com");

    let by_name = service
        .list_customers(Pagination::default(), Some("bob"))
        .await
        .unwrap();
    assert_eq!(by_name.count, 1);
    assert_eq!(by_name.data[0].first_name.as_deref(), Some("Bob"));
}

#[tokio::test]
async fn duplicate_email_conflicts() {
    let (service, _) = service().await;
    service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();
    let error = service
        .create_customer(CreateCustomerInput::new("OWNER@example.com"))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn invalid_email_is_rejected() {
    let (service, _) = service().await;
    let error = service
        .create_customer(CreateCustomerInput::new("not-an-email"))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn soft_deleted_customers_disappear() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("gone@example.com"))
        .await
        .unwrap();
    let second = service
        .create_customer(CreateCustomerInput::new("kept@example.com"))
        .await
        .unwrap();

    service.delete_customer(&customer.id).await.unwrap();

    assert!(service.get_customer(&customer.id).await.is_err());
    assert!(
        service
            .get_customer_by_email("gone@example.com")
            .await
            .unwrap()
            .is_none()
    );
    let page = service
        .list_customers(Pagination::default(), None)
        .await
        .unwrap();
    assert_eq!(page.count, 1);
    assert_eq!(page.data[0].id, second.id);
    assert!(service.delete_customer(&customer.id).await.is_err());
}

#[tokio::test]
async fn guest_upsert_fills_missing_names_only() {
    let (service, _) = service().await;
    let guest = service
        .create_customer(CreateCustomerInput::new("guest@example.com"))
        .await
        .unwrap();

    let filled = service
        .find_or_create_guest("GUEST@example.com", Some("Ada"), Some("Lovelace"))
        .await
        .unwrap();
    assert_eq!(filled.id, guest.id);
    assert_eq!(filled.first_name.as_deref(), Some("Ada"));
    assert!(!filled.has_account);

    let unchanged = service
        .find_or_create_guest("guest@example.com", Some("Grace"), Some("Hopper"))
        .await
        .unwrap();
    assert_eq!(unchanged.first_name.as_deref(), Some("Ada"));
    assert_eq!(unchanged.last_name.as_deref(), Some("Lovelace"));

    let account = service
        .register(
            RegisterCustomerInput::new("account@example.com", "hunter2hunter2")
                .with_name("Account", "Owner"),
        )
        .await
        .unwrap();
    let untouched = service
        .find_or_create_guest("account@example.com", Some("Other"), Some("Name"))
        .await
        .unwrap();
    assert_eq!(untouched.id, account.id);
    assert_eq!(untouched.first_name.as_deref(), Some("Account"));
    assert!(untouched.has_account);
}

#[tokio::test]
async fn duplicate_registration_conflicts() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let error = service
        .register(RegisterCustomerInput::new(
            "OWNER@example.com",
            "other-password",
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn customer_validation_bounds_input() {
    let (service, _) = service().await;

    let long_name = "a".repeat(MAX_NAME_LENGTH + 1);
    let error = service
        .create_customer(CreateCustomerInput::new("owner@example.com").with_name(long_name, "B"))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let long_phone = "1".repeat(MAX_PHONE_LENGTH + 1);
    let error = service
        .create_customer(CreateCustomerInput::new("owner@example.com").with_phone(long_phone))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .register(RegisterCustomerInput::new("owner@example.com", "short"))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}
