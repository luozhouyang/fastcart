use super::*;

#[utoipa::path(
    post,
    path = "/checkouts",
    tag = "Store",
    summary = "Create a checkout from a cart",
    request_body = CreateCheckoutRequestDto,
    security(("customerSession" = []), ()),
    responses((status = 201, body = CheckoutResponse))
)]
pub(super) async fn create_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCheckoutRequestDto>,
) -> ApiResult<(StatusCode, Json<CheckoutResponse>)> {
    if !context
        .checkout_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let cart_id = parse_cart_id(&request.cart_id)?;
    let carts = CartWorkflow::new(db.clone());
    let cart = carts.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let cart_context = cart_context(
        &db,
        &actor,
        &cart,
        request.country_code,
        request.province_code,
    )
    .await?;
    let settings = StoreService::new(db.clone()).get_all_settings().await?;
    validate_checkout_settings(
        &settings,
        cart.customer_id.as_ref(),
        cart.shipping_address.phone.as_deref(),
    )
    .map_err(ApiError::validation)?;
    let payment =
        PaymentService::with_registry(db.clone(), context.payment_providers_for(&store.store_id));
    let workflow = CheckoutWorkflow::new(db.clone(), payment);
    let mut checkout_request =
        CreateCheckoutRequest::new(cart_id, request.payment_provider, cart_context);
    checkout_request.email = request.email;
    checkout_request.idempotency_key = request.idempotency_key;
    if let Some(expires) = request.expires_in_seconds {
        checkout_request.expires_in_seconds = expires;
    }
    let checkout = workflow.create(checkout_request).await?;
    Ok((StatusCode::CREATED, Json(CheckoutResponse { checkout })))
}

#[utoipa::path(
    get,
    path = "/checkouts/{checkout_id}",
    tag = "Store",
    summary = "Get a checkout",
    security(("customerSession" = []), ()),
    responses((status = 200, body = CheckoutResponse))
)]
pub(super) async fn get_checkout(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<CheckoutResponse>> {
    let service = checkout_service(db.clone());
    let checkout = service
        .get(&fastcart_kernel::CheckoutId::from_string(checkout_id))
        .await?;
    let cart = CartWorkflow::new(db).get_cart(&checkout.cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(CheckoutResponse { checkout }))
}

#[utoipa::path(
    post,
    path = "/checkouts/{checkout_id}/complete",
    tag = "Store",
    summary = "Complete a checkout",
    request_body = CompleteCheckoutRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CheckoutCompleteResponse))
)]
pub(super) async fn complete_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
    ApiJson(request): ApiJson<CompleteCheckoutRequest>,
) -> ApiResult<Json<CheckoutCompleteResponse>> {
    if !context
        .checkout_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let checkout_id = fastcart_kernel::CheckoutId::from_string(checkout_id);
    let payment =
        PaymentService::with_registry(db.clone(), context.payment_providers_for(&store.store_id));
    let workflow = CheckoutWorkflow::new(db.clone(), payment);
    let checkout = workflow.get(&checkout_id).await?;
    let cart = CartWorkflow::new(db.clone())
        .get_cart(&checkout.cart_id)
        .await?;
    ensure_cart_access(&cart, &actor)?;
    let cart_context = cart_context(
        &db,
        &actor,
        &cart,
        request.country_code,
        request.province_code,
    )
    .await?;
    let result = workflow.complete(&checkout_id, &cart_context).await?;
    // The order is already committed; a settings read failure must not turn a
    // successful checkout into an error. Default to sending the email.
    let confirmation_emails = match StoreService::new(db.clone())
        .get_setting("confirmation_emails")
        .await
    {
        Ok(Some(value)) => value != "false",
        Ok(None) => true,
        Err(error) => {
            tracing::warn!(order_id = %result.order.id, error = %error, "failed to read confirmation email setting");
            true
        }
    };
    if confirmation_emails {
        let message = EmailMessage::new(
            result.order.email.clone(),
            format!("Order #{} confirmed", result.order.display_number),
        )
        .with_text(format!(
            "Thank you for your order. Total: {} {}.",
            result.order.total, result.order.currency_code
        ));
        if let Err(error) = context
            .email
            .enqueue(&JobQueue::new(db.pool().clone()), message, Some(10))
            .await
        {
            tracing::warn!(order_id = %result.order.id, error = %error, "failed to enqueue order confirmation email");
        }
    }
    Ok(Json(CheckoutCompleteResponse::order(result.order)))
}

#[utoipa::path(
    post,
    path = "/checkouts/{checkout_id}/cancel",
    tag = "Store",
    summary = "Cancel a checkout",
    security(("customerSession" = []), ()),
    responses((status = 200, body = CheckoutResponse))
)]
pub(super) async fn cancel_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<CheckoutResponse>> {
    let checkout_id = fastcart_kernel::CheckoutId::from_string(checkout_id);
    let service = checkout_service(db.clone());
    let checkout = service.get(&checkout_id).await?;
    let cart = CartWorkflow::new(db.clone())
        .get_cart(&checkout.cart_id)
        .await?;
    ensure_cart_access(&cart, &actor)?;
    let checkout = if checkout.status == CheckoutStatus::Canceled {
        checkout
    } else {
        // Claim the cancellation before touching payment or stock. A concurrent
        // completion then either wins the state transition or fails cleanly.
        service
            .transition(
                &checkout_id,
                CheckoutStatus::Canceled,
                Some("canceled by customer"),
            )
            .await?
    };
    if let Some(session_id) = checkout.payment_session_id.as_ref() {
        let payment = PaymentService::with_registry(
            db.clone(),
            context.payment_providers_for(&store.store_id),
        );
        if let Err(error) = payment.cancel_session(session_id.as_str()).await {
            tracing::warn!(checkout_id = %checkout_id, error = %error, "failed to cancel payment session");
        }
    }
    if let Err(error) = InventoryService::new(db)
        .release_checkout(&checkout_id)
        .await
    {
        tracing::warn!(checkout_id = %checkout_id, error = %error, "failed to release checkout reservations");
    }
    Ok(Json(CheckoutResponse { checkout }))
}

#[utoipa::path(
    get,
    path = "/checkouts/{checkout_id}/history",
    tag = "Store",
    summary = "List checkout state history",
    security(("customerSession" = []), ()),
    responses((status = 200, body = CheckoutHistoryResponse))
)]
pub(super) async fn checkout_history(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<CheckoutHistoryResponse>> {
    let checkout_id = fastcart_kernel::CheckoutId::from_string(checkout_id);
    let service = checkout_service(db.clone());
    let checkout = service.get(&checkout_id).await?;
    let cart = CartWorkflow::new(db).get_cart(&checkout.cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(CheckoutHistoryResponse {
        history: service.list_history(&checkout_id).await?,
    }))
}
