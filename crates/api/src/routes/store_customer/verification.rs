use super::*;

/// Requests an email-verification token.
///
/// The response is always the same 202 acknowledgement, whether or not the
/// account exists or is already verified. Email delivery failures are logged
/// and never change the response.
#[utoipa::path(
    post,
    path = "/customers/email-verification/request",
    tag = "Store",
    summary = "Request a customer email-verification token",
    request_body = EmailRequest,
    responses((status = 202, body = AcceptedResponse))
)]
pub(super) async fn request_email_verification(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<EmailRequest>,
) -> ApiResult<(StatusCode, Json<AcceptedResponse>)> {
    let email = normalize_email(&request.email);
    let key = format!("email-verification:{email}");
    if !context.customer_email_limiter.check(&key) {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let service = CustomerService::new(db.clone());
    if let Some(customer) = service.get_customer_by_email(&email).await? {
        match service.request_email_verification(&customer.id).await {
            Ok(created) => {
                enqueue_auth_email(
                    &context,
                    &db,
                    &customer.email,
                    "Verify your email address",
                    format!(
                        "Use this token to verify your email address: {}",
                        created.token
                    ),
                )
                .await;
            }
            // Already verified and recently deleted accounts stay silent.
            Err(AppError::Conflict(_) | AppError::NotFound(_)) => {}
            Err(error) => return Err(ApiError::new(error)),
        }
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(AcceptedResponse { accepted: true }),
    ))
}

/// Confirms an email-verification token.
#[utoipa::path(
    post,
    path = "/customers/email-verification/confirm",
    tag = "Store",
    summary = "Confirm a customer email-verification token",
    request_body = TokenRequest,
    responses((status = 200, body = EmailVerifiedResponse))
)]
pub(super) async fn confirm_email_verification(
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<TokenRequest>,
) -> ApiResult<Json<EmailVerifiedResponse>> {
    CustomerService::new(db)
        .verify_email(&request.token)
        .await?;
    Ok(Json(EmailVerifiedResponse {
        email_verified: true,
    }))
}

/// Requests a password-reset token.
///
/// Unknown and guest emails do not create tokens. The response is always the
/// same 202 acknowledgement and email failures are only logged.
#[utoipa::path(
    post,
    path = "/customers/password-reset/request",
    tag = "Store",
    summary = "Request a customer password-reset token",
    request_body = PasswordResetRequest,
    responses((status = 202, body = AcceptedResponse))
)]
pub(super) async fn request_password_reset(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<PasswordResetRequest>,
) -> ApiResult<(StatusCode, Json<AcceptedResponse>)> {
    let email = normalize_email(&request.email);
    let key = format!("password-reset:{email}");
    if !context.customer_email_limiter.check(&key) {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let service = CustomerService::new(db.clone());
    if let Some(created) = service.request_password_reset(&email).await? {
        enqueue_auth_email(
            &context,
            &db,
            &email,
            "Reset your password",
            format!("Use this token to reset your password: {}", created.token),
        )
        .await;
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(AcceptedResponse { accepted: true }),
    ))
}

/// Confirms a password-reset token and revokes every session.
#[utoipa::path(
    post,
    path = "/customers/password-reset/confirm",
    tag = "Store",
    summary = "Confirm a customer password-reset token",
    request_body = ResetPasswordRequest,
    responses((status = 204))
)]
pub(super) async fn confirm_password_reset(
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<ResetPasswordRequest>,
) -> ApiResult<StatusCode> {
    CustomerService::new(db)
        .reset_password(&request.token, &request.password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
