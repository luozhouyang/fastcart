use super::*;

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/email",
    tag = "Store",
    summary = "Set the cart contact email",
    request_body = SetEmailRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn set_email(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetEmailRequest>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db);
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(CartResponse {
        cart: workflow
            .cart_service()
            .set_email(&cart_id, &request.email)
            .await?,
    }))
}

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/customer",
    tag = "Store",
    summary = "Attach the authenticated customer to a cart",
    security(("customerSession" = [])),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn attach_customer(
    State(_context): State<Arc<AppContext>>,
    CustomerActor(actor): CustomerActor,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let workflow = CartWorkflow::new(db);
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(CartResponse {
        cart: workflow
            .cart_service()
            .attach_customer(&cart_id, &customer_id, None)
            .await?,
    }))
}

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/addresses",
    tag = "Store",
    summary = "Set a cart shipping or billing address",
    request_body = SetAddressRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn set_address(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetAddressRequest>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    if request.address_type == "shipping" {
        // Billing addresses are not constrained: regions model destinations.
        ensure_region_country(&db, &cart, request.country_code.as_deref()).await?;
    }
    let input = AddressInput {
        first_name: request.first_name.clone(),
        last_name: request.last_name.clone(),
        company: request.company.clone(),
        address_1: request.address_1.clone(),
        address_2: request.address_2.clone(),
        city: request.city.clone(),
        province: request.province.clone(),
        postal_code: request.postal_code.clone(),
        country_code: request.country_code.clone(),
        phone: request.phone.clone(),
        metadata: Metadata::empty(),
    };
    match request.address_type.as_str() {
        "shipping" => {
            workflow
                .cart_service()
                .set_shipping_address(&cart_id, input)
                .await?;
        }
        "billing" => {
            workflow
                .cart_service()
                .set_billing_address(&cart_id, input)
                .await?;
        }
        other => {
            return Err(ApiError::validation(format!(
                "address type must be shipping or billing, got {other}"
            )));
        }
    }
    let cart = workflow.get_cart(&cart_id).await?;
    let context = cart_context(
        &db,
        &actor,
        &cart,
        request.country_code.clone(),
        request.province.clone(),
    )
    .await?;
    Ok(Json(CartResponse {
        cart: workflow.recalculate(&cart_id, &context).await?,
    }))
}

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/shipping-method",
    tag = "Store",
    summary = "Set the cart shipping method",
    request_body = SetShippingMethodRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn set_shipping_method(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetShippingMethodRequest>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let context = cart_context(
        &db,
        &actor,
        &cart,
        request.country_code,
        request.province_code,
    )
    .await?;
    let cart = workflow
        .set_shipping_method(
            &cart_id,
            &ShippingOptionId::from_string(request.shipping_option_id),
            &context,
        )
        .await?;
    Ok(Json(CartResponse { cart }))
}

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/discounts",
    tag = "Store",
    summary = "Apply a discount code to a cart",
    params(DestinationQuery),
    request_body = DiscountRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn apply_discount(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
    ApiJson(request): ApiJson<DiscountRequest>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(CartResponse {
        cart: workflow
            .apply_coupon(&cart_id, &request.code, &context)
            .await?,
    }))
}

#[utoipa::path(
    delete,
    path = "/carts/{cart_id}/discounts/{code}",
    tag = "Store",
    summary = "Remove a discount code from a cart",
    params(DestinationQuery),
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn remove_discount(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path((cart_id, code)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<CartResponse>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(CartResponse {
        cart: workflow.remove_coupon(&cart_id, &code, &context).await?,
    }))
}
