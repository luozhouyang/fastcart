use super::*;

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
