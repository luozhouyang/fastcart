use super::*;

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
