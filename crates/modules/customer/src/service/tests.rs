use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, CountryCode, CustomerId, Metadata, Pagination, now_timestamp};

use crate::domain::{
    AddressInput, CreateCustomerInput, CustomerAuthTokenType, CustomerStatus, MAX_NAME_LENGTH,
    MAX_PHONE_LENGTH, RegisterCustomerInput, UpdateCustomerInput,
};

use fastcart_kernel::StoreId;

use super::*;

async fn service() -> (CustomerService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    (CustomerService::new(db.clone()), db)
}

fn address(address_1: &str) -> AddressInput {
    AddressInput::new(address_1, "Springfield", "12345", "us")
}

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
async fn registration_with_session_is_authenticatable() {
    let (service, _) = service().await;
    let session = service
        .register_with_session(RegisterCustomerInput::new("owner@example.com", "password"))
        .await
        .unwrap();

    assert!(session.customer.has_account);
    assert_eq!(session.customer.email, "owner@example.com");

    let authenticated = service.authenticate_session(&session.token).await.unwrap();
    assert_eq!(authenticated.customer.id, session.customer.id);
    assert_eq!(authenticated.session_id, session.session_id);
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

#[tokio::test]
async fn groups_are_created_assigned_and_removed() {
    let (service, _) = service().await;
    let customer = service
        .create_customer(CreateCustomerInput::new("owner@example.com"))
        .await
        .unwrap();
    let group = service
        .create_group("VIP", Metadata::empty())
        .await
        .unwrap();
    assert_eq!(group.name, "VIP");

    let duplicate = service
        .create_group("VIP", Metadata::empty())
        .await
        .unwrap_err();
    assert!(matches!(duplicate, AppError::Conflict(_)));

    service.assign_group(&customer.id, &group.id).await.unwrap();
    service.assign_group(&customer.id, &group.id).await.unwrap();
    let assigned = service.list_customer_groups(&customer.id).await.unwrap();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].id, group.id);

    assert_eq!(service.get_group(&group.id).await.unwrap(), group);
    assert_eq!(service.list_groups().await.unwrap().len(), 1);

    service.remove_group(&customer.id, &group.id).await.unwrap();
    assert!(
        service
            .list_customer_groups(&customer.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Argon2 must not run on the async worker: a single-threaded runtime has
/// to keep polling other tasks while the (deliberately slow) hash is
/// computed, otherwise one login attempt starves the whole worker.
#[tokio::test(flavor = "current_thread")]
async fn login_does_not_block_the_async_runtime() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();

    // The watchdog ticks every millisecond and records the longest time it
    // went unscheduled. Database round trips are sub-millisecond, so any
    // multi-millisecond gap is the Argon2 call being run inline.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let max_gap_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let watchdog = tokio::spawn({
        let stop = stop.clone();
        let max_gap_ms = max_gap_ms.clone();
        async move {
            let mut last = tokio::time::Instant::now();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                let now = tokio::time::Instant::now();
                max_gap_ms.fetch_max(
                    u64::try_from((now - last).as_millis()).unwrap_or(u64::MAX),
                    std::sync::atomic::Ordering::Relaxed,
                );
                last = now;
            }
        }
    });
    tokio::task::yield_now().await;

    service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    watchdog.await.unwrap();

    let gap_ms = max_gap_ms.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        gap_ms < 20,
        "the async runtime was blocked for {gap_ms}ms while Argon2 verified a password"
    );
}

#[tokio::test]
async fn registration_login_and_session_authentication() {
    let (service, db) = service().await;
    let customer = service
        .register(
            RegisterCustomerInput::new("Owner@Example.com", "hunter2hunter2")
                .with_name("Ada", "Lovelace"),
        )
        .await
        .unwrap();
    assert!(customer.has_account);
    assert_eq!(customer.email, "owner@example.com");

    let stored_hash: String =
        sqlx::query_scalar("SELECT password_hash FROM customer_credentials WHERE customer_id = ?")
            .bind(customer.id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(stored_hash.starts_with("$argon2"));
    assert_ne!(stored_hash, "hunter2hunter2");

    let session = service
        .login("OWNER@example.com", "hunter2hunter2")
        .await
        .unwrap();
    assert_eq!(session.customer.id, customer.id);
    assert_eq!(session.token.len(), 64);
    assert!(!session.csrf_token.is_empty());

    let stored_token_hash: String =
        sqlx::query_scalar("SELECT token_hash FROM customer_sessions WHERE id = ?")
            .bind(session.session_id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(stored_token_hash, hash_token(&session.token));
    assert_ne!(stored_token_hash, session.token);

    let debug = format!("{session:?}");
    assert!(!debug.contains(&session.token));
    assert!(!debug.contains("hunter2"));

    let authenticated = service.authenticate_session(&session.token).await.unwrap();
    assert_eq!(authenticated.customer.id, customer.id);
    assert_eq!(authenticated.session_id, session.session_id);
}

#[tokio::test]
async fn session_customer_lookup_is_read_only() {
    let (service, db) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    // A sentinel proves the lookup never touches last_used_at, even when
    // the call happens within the same clock tick as session creation.
    sqlx::query("UPDATE customer_sessions SET last_used_at = 1 WHERE id = ?")
        .bind(session.session_id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let found = service
        .session_customer(&session.token)
        .await
        .unwrap()
        .expect("valid session resolves to its customer");
    assert_eq!(found.id, customer.id);
    assert_eq!(found.email, customer.email);
    assert!(found.has_account);

    let last_used_at: i64 =
        sqlx::query_scalar("SELECT last_used_at FROM customer_sessions WHERE id = ?")
            .bind(session.session_id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(last_used_at, 1);

    // The side-effecting variant still refreshes the timestamp.
    service.authenticate_session(&session.token).await.unwrap();
    let refreshed: i64 =
        sqlx::query_scalar("SELECT last_used_at FROM customer_sessions WHERE id = ?")
            .bind(session.session_id.as_str())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(refreshed > 1);
}

#[tokio::test]
async fn session_customer_lookup_rejects_invalid_sessions() {
    let (service, db) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();

    // Unknown and too-short tokens resolve to None rather than an error.
    assert!(
        service
            .session_customer(&"a".repeat(64))
            .await
            .unwrap()
            .is_none()
    );
    assert!(service.session_customer("short").await.unwrap().is_none());

    // Revoked session.
    let revoked = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    service.logout(&revoked.token).await.unwrap();
    assert!(
        service
            .session_customer(&revoked.token)
            .await
            .unwrap()
            .is_none()
    );

    // Expired session.
    let expired = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    sqlx::query("UPDATE customer_sessions SET expires_at = 0 WHERE id = ?")
        .bind(expired.session_id.as_str())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        service
            .session_customer(&expired.token)
            .await
            .unwrap()
            .is_none()
    );

    // Disabled customer.
    let disabled = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    service
        .update_customer(
            &disabled.customer.id,
            UpdateCustomerInput {
                status: Some(CustomerStatus::Disabled),
                ..UpdateCustomerInput::default()
            },
        )
        .await
        .unwrap();
    assert!(
        service
            .session_customer(&disabled.token)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn session_customer_lookup_rejects_soft_deleted_customers() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    service.delete_customer(&customer.id).await.unwrap();

    assert!(
        service
            .session_customer(&session.token)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn wrong_password_and_unknown_email_are_unauthorized() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();

    let error = service
        .login("owner@example.com", "wrong-password")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Unauthorized));

    let error = service
        .login("missing@example.com", "hunter2hunter2")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Unauthorized));
}

#[tokio::test]
async fn logout_revokes_the_session() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    service.logout(&session.token).await.unwrap();
    assert!(service.authenticate_session(&session.token).await.is_err());
}

#[tokio::test]
async fn expired_sessions_are_rejected() {
    let (service, db) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    sqlx::query("UPDATE customer_sessions SET expires_at = 0 WHERE id = ?")
        .bind(session.session_id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    assert!(service.authenticate_session(&session.token).await.is_err());
}

#[tokio::test]
async fn changing_password_revokes_sessions() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    service
        .change_password(&customer.id, "hunter2hunter2", "new-password-1")
        .await
        .unwrap();

    assert!(service.authenticate_session(&session.token).await.is_err());
    assert!(
        service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .is_err()
    );
    assert!(
        service
            .login("owner@example.com", "new-password-1")
            .await
            .is_ok()
    );

    let wrong_current = service
        .change_password(&customer.id, "nope-nope", "another-password")
        .await
        .unwrap_err();
    assert!(matches!(wrong_current, AppError::Unauthorized));

    let short = service
        .change_password(&customer.id, "new-password-1", "short")
        .await
        .unwrap_err();
    assert!(matches!(short, AppError::Validation(_)));
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
async fn deleted_customers_cannot_authenticate() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    service.delete_customer(&customer.id).await.unwrap();

    assert!(service.authenticate_session(&session.token).await.is_err());
    assert!(
        service
            .login("owner@example.com", "hunter2hunter2")
            .await
            .is_err()
    );
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

#[tokio::test]
async fn sessions_are_listed_without_token_digests() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "hunter2hunter2",
        ))
        .await
        .unwrap();
    let first = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();
    let second = service
        .login("owner@example.com", "hunter2hunter2")
        .await
        .unwrap();

    let sessions = service.list_sessions(&customer.id).await.unwrap();
    assert_eq!(sessions.len(), 2);

    service.revoke_all_sessions(&customer.id).await.unwrap();
    assert!(service.authenticate_session(&first.token).await.is_err());
    assert!(service.authenticate_session(&second.token).await.is_err());

    let sessions = service.list_sessions(&customer.id).await.unwrap();
    assert!(sessions.iter().all(|session| session.revoked_at.is_some()));
}

#[test]
fn auth_token_type_round_trips() {
    assert_eq!(
        CustomerAuthTokenType::parse("email_verification"),
        Some(CustomerAuthTokenType::EmailVerification)
    );
    assert_eq!(
        CustomerAuthTokenType::parse("password_reset"),
        Some(CustomerAuthTokenType::PasswordReset)
    );
    assert_eq!(CustomerAuthTokenType::parse("other"), None);
    assert_eq!(
        CustomerAuthTokenType::EmailVerification.as_str(),
        "email_verification"
    );
    assert_eq!(
        CustomerAuthTokenType::PasswordReset.as_str(),
        "password_reset"
    );
}

#[tokio::test]
async fn email_verification_token_is_hashed_and_single_use() {
    let (service, db) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    assert!(customer.email_verified_at.is_none());

    let created = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();
    assert_eq!(created.token.len(), 64);
    assert!(created.expires_at > now_timestamp());
    let debug = format!("{created:?}");
    assert!(!debug.contains(&created.token));

    let (stored_hash, token_type, used_at): (String, String, Option<i64>) = sqlx::query_as(
        "SELECT token_hash, token_type, used_at FROM customer_auth_tokens
             WHERE customer_id = ?",
    )
    .bind(customer.id.as_str())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(stored_hash, hash_token(&created.token));
    assert_ne!(stored_hash, created.token);
    assert_eq!(token_type, "email_verification");
    assert!(used_at.is_none());

    let verified = service.verify_email(&created.token).await.unwrap();
    assert!(verified.email_verified_at.is_some());

    let reused = service.verify_email(&created.token).await.unwrap_err();
    assert!(matches!(reused, AppError::Validation(_)));
}

#[tokio::test]
async fn verification_token_is_consumed_once_under_concurrency() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let created = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();

    let (first, second) = tokio::join!(
        service.verify_email(&created.token),
        service.verify_email(&created.token)
    );
    assert!(
        first.is_ok() ^ second.is_ok(),
        "exactly one confirmation must succeed"
    );
}

#[tokio::test]
async fn verification_rejects_expired_tokens() {
    let (service, db) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let created = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();

    sqlx::query("UPDATE customer_auth_tokens SET expires_at = 0 WHERE customer_id = ?")
        .bind(customer.id.as_str())
        .execute(db.pool())
        .await
        .unwrap();

    let error = service.verify_email(&created.token).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    let current = service.get_customer(&customer.id).await.unwrap();
    assert!(current.email_verified_at.is_none());
}

#[tokio::test]
async fn already_verified_customers_cannot_request_again() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let created = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();
    service.verify_email(&created.token).await.unwrap();

    let error = service
        .request_email_verification(&customer.id)
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn requesting_verification_invalidates_previous_tokens() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let first = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();
    let second = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();

    assert!(service.verify_email(&first.token).await.is_err());
    assert!(service.verify_email(&second.token).await.is_ok());
}

#[tokio::test]
async fn auth_tokens_reject_wrong_type() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();

    let verification = service
        .request_email_verification(&customer.id)
        .await
        .unwrap();
    let error = service
        .reset_password(&verification.token, "new-reset-password-1")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let reset = service
        .request_password_reset("owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let error = service.verify_email(&reset.token).await.unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn reset_password_changes_login_and_revokes_sessions() {
    let (service, _) = service().await;
    let customer = service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let session = service
        .login("owner@example.com", "reset-password-1")
        .await
        .unwrap();

    let reset = service
        .request_password_reset("OWNER@Example.com")
        .await
        .unwrap()
        .expect("existing account receives a reset token");
    assert_eq!(reset.token.len(), 64);
    assert!(reset.expires_at > now_timestamp());

    let updated = service
        .reset_password(&reset.token, "new-reset-password-1")
        .await
        .unwrap();
    assert_eq!(updated.id, customer.id);
    assert!(service.authenticate_session(&session.token).await.is_err());
    assert!(
        service
            .login("owner@example.com", "reset-password-1")
            .await
            .is_err()
    );
    assert!(
        service
            .login("owner@example.com", "new-reset-password-1")
            .await
            .is_ok()
    );
    assert!(
        service
            .reset_password(&reset.token, "another-reset-password-1")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn reset_password_rejects_short_passwords_without_consuming_the_token() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let reset = service
        .request_password_reset("owner@example.com")
        .await
        .unwrap()
        .unwrap();

    let error = service
        .reset_password(&reset.token, "short")
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(
        service
            .reset_password(&reset.token, "another-reset-password-1")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn requesting_reset_invalidates_previous_tokens() {
    let (service, _) = service().await;
    service
        .register(RegisterCustomerInput::new(
            "owner@example.com",
            "reset-password-1",
        ))
        .await
        .unwrap();
    let first = service
        .request_password_reset("owner@example.com")
        .await
        .unwrap()
        .unwrap();
    let second = service
        .request_password_reset("owner@example.com")
        .await
        .unwrap()
        .unwrap();

    assert!(
        service
            .reset_password(&first.token, "new-reset-password-1")
            .await
            .is_err()
    );
    assert!(
        service
            .reset_password(&second.token, "new-reset-password-1")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn reset_request_for_unknown_or_guest_email_returns_none() {
    let (service, _) = service().await;
    assert!(
        service
            .request_password_reset("missing@example.com")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .request_password_reset("  MISSING@Example.com ")
            .await
            .unwrap()
            .is_none()
    );
    service
        .create_customer(CreateCustomerInput::new("guest@example.com"))
        .await
        .unwrap();
    assert!(
        service
            .request_password_reset("guest@example.com")
            .await
            .unwrap()
            .is_none()
    );
}
