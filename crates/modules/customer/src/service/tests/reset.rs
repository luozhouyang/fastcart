use super::*;

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
