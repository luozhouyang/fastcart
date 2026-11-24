use super::*;

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
