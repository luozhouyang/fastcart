use super::*;

#[utoipa::path(
    post,
    path = "/carts",
    tag = "Store",
    summary = "Create a cart",
    request_body = CreateCartRequest,
    security(("customerSession" = []), ()),
    responses((status = 201, body = CartResponse))
)]
pub(super) async fn create_cart(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    publishable_key: Option<Extension<crate::routes::PublishableKeyContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCartRequest>,
) -> ApiResult<(StatusCode, Json<CartResponse>)> {
    if !context
        .cart_create_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }
    let mut sales_channel_id = request.sales_channel_id;
    if let Some(Extension(key)) = publishable_key {
        match sales_channel_id.as_deref() {
            Some(channel_id) if key.sales_channel_ids.iter().any(|id| id == channel_id) => {}
            Some(_) => {
                return Err(ApiError::new(AppError::Forbidden));
            }
            None => {
                sales_channel_id = key.sales_channel_ids.first().cloned();
            }
        }
    }
    if let Some(channel_id) = sales_channel_id.as_deref() {
        let channel = StoreService::new(db.clone())
            .get_sales_channel(channel_id)
            .await?;
        if channel.is_disabled {
            return Err(ApiError::validation("sales channel is disabled"));
        }
    }
    let currency = CurrencyCode::new(request.currency_code.clone())
        .map(|currency| currency.as_str().to_owned())
        .map_err(|error| ApiError::new(AppError::from(error)))?;
    let region_id = resolve_cart_region(&db, request.region_id, &currency).await?;
    let mut input = CreateCartInput::new(currency);
    input.email = request.email;
    input.sales_channel_id = sales_channel_id;
    input.region_id = region_id;
    input.expires_in_seconds = request.expires_in_seconds;
    input.customer_id = actor.customer_id.clone();
    let cart = CartWorkflow::new(db).create_cart(input).await?;
    Ok((StatusCode::CREATED, Json(CartResponse { cart })))
}

/// Resolves the region a new cart should be attached to.
///
/// Rules, in order:
///
/// 1. An explicit `region_id` must exist and its currency must equal the cart
///    currency; a mismatch is rejected instead of silently repricing the cart.
/// 2. Without `region_id`, the store's default region is attached only when its
///    currency matches the requested cart currency. A default region in
///    another currency is ignored, preserving pre-region behavior for clients
///    that only send `currency_code`.
/// 3. Otherwise the cart keeps no region.
pub(super) async fn resolve_cart_region(
    db: &StoreDatabase,
    requested: Option<String>,
    currency: &str,
) -> ApiResult<Option<String>> {
    let regions = RegionService::new(db.clone());
    let Some(region_id) = requested else {
        return Ok(match regions.default_region().await? {
            Some(region) if region.currency_code == currency => Some(region.id),
            _ => None,
        });
    };
    let region = regions.get_region(&region_id).await?;
    if region.currency_code != currency {
        return Err(ApiError::validation(format!(
            "cart currency {currency} does not match region {} currency {}",
            region.id, region.currency_code
        )));
    }
    Ok(Some(region.id))
}

#[utoipa::path(
    get,
    path = "/carts/{cart_id}",
    tag = "Store",
    summary = "Get a cart",
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn get_cart(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
) -> ApiResult<Json<CartResponse>> {
    let cart = CartWorkflow::new(db)
        .get_cart(&parse_cart_id(&cart_id)?)
        .await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(CartResponse { cart }))
}

#[utoipa::path(
    post,
    path = "/carts/{cart_id}/items",
    tag = "Store",
    summary = "Add an item to a cart",
    request_body = AddItemRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn add_item(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<AddItemRequest>,
) -> ApiResult<Json<CartResponse>> {
    if !context
        .cart_write_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let destination = request
        .country_code
        .as_deref()
        .or(cart.shipping_address.country_code.as_deref());
    ensure_region_country(&db, &cart, destination).await?;
    let context = cart_context(
        &db,
        &actor,
        &cart,
        request.country_code,
        request.province_code,
    )
    .await?;
    let cart = workflow
        .add_item(
            &cart_id,
            &VariantId::from_string(request.variant_id),
            request.quantity,
            &context,
        )
        .await?;
    Ok(Json(CartResponse { cart }))
}

#[utoipa::path(
    patch,
    path = "/carts/{cart_id}/items/{item_id}",
    tag = "Store",
    summary = "Update a cart item quantity",
    params(DestinationQuery),
    request_body = UpdateItemRequest,
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn update_item(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreDb(db): StoreDb,
    Path((cart_id, item_id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
    ApiJson(request): ApiJson<UpdateItemRequest>,
) -> ApiResult<Json<CartResponse>> {
    if !context
        .cart_write_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    workflow
        .cart_service()
        .update_item_quantity(
            &cart_id,
            &LineItemId::from_string(item_id),
            UpdateItemInput {
                quantity: request.quantity,
            },
        )
        .await?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(CartResponse {
        cart: workflow.recalculate(&cart_id, &context).await?,
    }))
}

#[utoipa::path(
    delete,
    path = "/carts/{cart_id}/items/{item_id}",
    tag = "Store",
    summary = "Remove a cart item",
    params(DestinationQuery),
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn remove_item(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreDb(db): StoreDb,
    Path((cart_id, item_id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<CartResponse>> {
    if !context
        .cart_write_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    workflow
        .cart_service()
        .remove_item(&cart_id, &LineItemId::from_string(item_id))
        .await?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(CartResponse {
        cart: workflow.recalculate(&cart_id, &context).await?,
    }))
}

#[utoipa::path(
    delete,
    path = "/carts/{cart_id}/items",
    tag = "Store",
    summary = "Clear cart items",
    params(DestinationQuery),
    security(("customerSession" = []), ()),
    responses((status = 200, body = CartResponse))
)]
pub(super) async fn clear_items(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    client_ip: Option<Extension<ClientIp>>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<CartResponse>> {
    if !context
        .cart_write_limiter
        .check(&client_key(client_ip.as_ref()))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    workflow.cart_service().clear_items(&cart_id).await?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(CartResponse {
        cart: workflow.recalculate(&cart_id, &context).await?,
    }))
}
