use super::*;

#[utoipa::path(
    post,
    path = "/customers",
    tag = "Store",
    summary = "Register a storefront customer",
    request_body = RegisterRequest,
    responses((status = 200, body = CustomerSessionResponse))
)]
pub(super) async fn register(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<RegisterRequest>,
) -> ApiResult<Response> {
    let email = request.email;
    let password = request.password;
    let service = CustomerService::new(db);
    service
        .register(RegisterCustomerInput {
            email: email.clone(),
            password: password.clone(),
            first_name: request.first_name,
            last_name: request.last_name,
            phone: None,
        })
        .await?;
    let session = service.login(&email, &password).await?;
    session_response(&context, session)
}

#[utoipa::path(
    post,
    path = "/customers/login",
    tag = "Store",
    summary = "Log in a storefront customer",
    request_body = CustomerLoginRequest,
    responses((status = 200, body = CustomerSessionResponse))
)]
pub(super) async fn login(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CustomerLoginRequest>,
) -> ApiResult<Response> {
    let email = normalize_email(&request.email);
    let key = format!("customer-login:{email}");
    if !context.login_limiter.check(&key) {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let session = CustomerService::new(db)
        .login(&email, &request.password)
        .await?;
    context.login_limiter.reset(&key);
    session_response(&context, session)
}

#[utoipa::path(
    post,
    path = "/customers/logout",
    tag = "Store",
    summary = "Log out the current storefront customer",
    security(("customerSession" = [])),
    responses((status = 204))
)]
pub(super) async fn logout(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    CustomerActor(_actor): CustomerActor,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let cookies = parse_cookies(&headers);
    let token = cookies
        .get(CUSTOMER_SESSION_COOKIE)
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    CustomerService::new(db).logout(token).await?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    clear_session_cookies(&context, &mut response)?;
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/customers/me",
    tag = "Store",
    summary = "Get the current customer",
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerResponse))
)]
pub(super) async fn me(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<CustomerResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let customer = CustomerService::new(db).get_customer(&customer_id).await?;
    Ok(Json(CustomerResponse {
        customer: customer.into(),
    }))
}

#[utoipa::path(
    patch,
    path = "/customers/me",
    tag = "Store",
    summary = "Update the current customer",
    request_body = UpdateProfileRequest,
    security(("customerSession" = [])),
    responses((status = 200, body = CustomerResponse))
)]
pub(super) async fn update_me(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    ApiJson(request): ApiJson<UpdateProfileRequest>,
) -> ApiResult<Json<CustomerResponse>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let customer = CustomerService::new(db)
        .update_customer(
            &customer_id,
            UpdateCustomerInput {
                email: None,
                first_name: request.first_name,
                last_name: request.last_name,
                phone: request.phone,
                status: None,
                metadata: None,
            },
        )
        .await?;
    Ok(Json(CustomerResponse {
        customer: customer.into(),
    }))
}

#[utoipa::path(
    post,
    path = "/customers/me/password",
    tag = "Store",
    summary = "Change the current customer's password",
    request_body = ChangePasswordRequest,
    security(("customerSession" = [])),
    responses((status = 204))
)]
pub(super) async fn change_password(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    ApiJson(request): ApiJson<ChangePasswordRequest>,
) -> ApiResult<Response> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    CustomerService::new(db)
        .change_password(
            &customer_id,
            &request.current_password,
            &request.new_password,
        )
        .await?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    clear_session_cookies(&context, &mut response)?;
    Ok(response)
}
