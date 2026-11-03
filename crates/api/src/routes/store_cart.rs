//! Storefront cart and checkout routes.
//!
//! Cart prices, discounts, tax and shipping are always produced by the cart
//! workflow from server-side data; request bodies never carry authoritative
//! amounts.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use fastcart_email::EmailMessage;
use fastcart_http::{ApiError, ApiJson, ApiQuery, ApiResult, CustomerActor};
use fastcart_infrastructure::{JobQueue, StoreDatabase};
use fastcart_kernel::{
    ActorContext, AppError, CartId, CountryCode, LineItemId, Metadata, ShippingOptionId, VariantId,
    now_timestamp,
};
use fastcart_module_cart::{AddressInput, Cart, CreateCartInput, UpdateItemInput};
use fastcart_module_checkout::{CheckoutService, CheckoutSession, CheckoutStatus};
use fastcart_module_customer::CustomerService;
use fastcart_module_inventory::InventoryService;
use fastcart_module_payment::PaymentService;
use fastcart_module_store::StoreService;
use fastcart_workflows::{CartContext, CartWorkflow, CheckoutWorkflow, CreateCheckoutRequest};

use super::{StoreDb, ensure_cart_access};
use crate::state::AppContext;

/// Builds cart and checkout routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/carts", post(create_cart))
        .route("/carts/{cart_id}", get(get_cart))
        .route("/carts/{cart_id}/items", post(add_item).delete(clear_items))
        .route(
            "/carts/{cart_id}/items/{item_id}",
            axum::routing::patch(update_item).delete(remove_item),
        )
        .route("/carts/{cart_id}/email", post(set_email))
        .route("/carts/{cart_id}/customer", post(attach_customer))
        .route("/carts/{cart_id}/addresses", post(set_address))
        .route(
            "/carts/{cart_id}/shipping-method",
            post(set_shipping_method),
        )
        .route("/carts/{cart_id}/discounts", post(apply_discount))
        .route("/carts/{cart_id}/discounts/{code}", delete(remove_discount))
        .route("/checkouts", post(create_checkout))
        .route("/checkouts/{checkout_id}", get(get_checkout))
        .route("/checkouts/{checkout_id}/complete", post(complete_checkout))
        .route("/checkouts/{checkout_id}/cancel", post(cancel_checkout))
        .route("/checkouts/{checkout_id}/history", get(checkout_history))
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreateCartRequest {
    /// Cart currency.
    pub currency_code: String,
    /// Optional contact email.
    pub email: Option<String>,
    /// Optional sales channel.
    pub sales_channel_id: Option<String>,
    /// Lifetime in seconds.
    pub expires_in_seconds: Option<i64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AddItemRequest {
    /// Variant to add.
    pub variant_id: String,
    /// Quantity.
    pub quantity: i64,
    /// Tax destination country.
    pub country_code: Option<String>,
    /// Tax destination province.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct UpdateItemRequest {
    /// New quantity; zero removes the line.
    pub quantity: i64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SetEmailRequest {
    /// Contact email.
    pub email: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SetAddressRequest {
    /// `shipping` or `billing`.
    #[serde(rename = "type")]
    pub address_type: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Company.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City.
    pub city: Option<String>,
    /// Province or state.
    pub province: Option<String>,
    /// Postal code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Phone number.
    pub phone: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SetShippingMethodRequest {
    /// Shipping option identifier.
    pub shipping_option_id: String,
    /// Destination country override.
    pub country_code: Option<String>,
    /// Destination province override.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DiscountRequest {
    /// Coupon code.
    pub code: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreateCheckoutRequestDto {
    /// Cart being checked out.
    pub cart_id: String,
    /// Optional contact email.
    pub email: Option<String>,
    /// Payment provider; defaults to `manual`.
    #[serde(default = "default_payment_provider")]
    pub payment_provider: String,
    /// Idempotency key.
    pub idempotency_key: Option<String>,
    /// Session lifetime.
    pub expires_in_seconds: Option<i64>,
    /// Tax destination country.
    pub country_code: Option<String>,
    /// Tax destination province.
    pub province_code: Option<String>,
}

fn default_payment_provider() -> String {
    "manual".to_owned()
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CompleteCheckoutRequest {
    /// Destination country override.
    pub country_code: Option<String>,
    /// Destination province override.
    pub province_code: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct CompleteCheckoutResponse {
    /// Final checkout.
    pub checkout: CheckoutSession,
    /// Placed order.
    pub order: fastcart_module_order::Order,
    /// Captured payment.
    pub payment: fastcart_module_payment::Payment,
}

async fn create_cart(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCartRequest>,
) -> ApiResult<(StatusCode, Json<Cart>)> {
    if let Some(channel_id) = request.sales_channel_id.as_deref() {
        let channel = StoreService::new(db.clone())
            .get_sales_channel(channel_id)
            .await?;
        if channel.is_disabled {
            return Err(ApiError::validation("sales channel is disabled"));
        }
    }
    let mut input = CreateCartInput::new(request.currency_code);
    input.email = request.email;
    input.sales_channel_id = request.sales_channel_id;
    input.expires_in_seconds = request.expires_in_seconds;
    input.customer_id = actor.customer_id.clone();
    let cart = CartWorkflow::new(db).create_cart(input).await?;
    Ok((StatusCode::CREATED, Json(cart)))
}

async fn get_cart(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
) -> ApiResult<Json<Cart>> {
    let cart = CartWorkflow::new(db)
        .get_cart(&parse_cart_id(&cart_id)?)
        .await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(cart))
}

async fn add_item(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<AddItemRequest>,
) -> ApiResult<Json<Cart>> {
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
        .add_item(
            &cart_id,
            &VariantId::from_string(request.variant_id),
            request.quantity,
            &context,
        )
        .await?;
    Ok(Json(cart))
}

async fn update_item(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path((cart_id, item_id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
    ApiJson(request): ApiJson<UpdateItemRequest>,
) -> ApiResult<Json<Cart>> {
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
    Ok(Json(workflow.recalculate(&cart_id, &context).await?))
}

async fn remove_item(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path((cart_id, item_id)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    workflow
        .cart_service()
        .remove_item(&cart_id, &LineItemId::from_string(item_id))
        .await?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(workflow.recalculate(&cart_id, &context).await?))
}

async fn clear_items(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    workflow.cart_service().clear_items(&cart_id).await?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(workflow.recalculate(&cart_id, &context).await?))
}

async fn set_email(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetEmailRequest>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db);
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(
        workflow
            .cart_service()
            .set_email(&cart_id, &request.email)
            .await?,
    ))
}

async fn attach_customer(
    State(_context): State<Arc<AppContext>>,
    CustomerActor(actor): CustomerActor,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let workflow = CartWorkflow::new(db);
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(
        workflow
            .cart_service()
            .attach_customer(&cart_id, &customer_id, None)
            .await?,
    ))
}

async fn set_address(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetAddressRequest>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
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
    Ok(Json(workflow.recalculate(&cart_id, &context).await?))
}

async fn set_shipping_method(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiJson(request): ApiJson<SetShippingMethodRequest>,
) -> ApiResult<Json<Cart>> {
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
    Ok(Json(cart))
}

async fn apply_discount(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(cart_id): Path<String>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
    ApiJson(request): ApiJson<DiscountRequest>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(
        workflow
            .apply_coupon(&cart_id, &request.code, &context)
            .await?,
    ))
}

async fn remove_discount(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path((cart_id, code)): Path<(String, String)>,
    ApiQuery(query): ApiQuery<DestinationQuery>,
) -> ApiResult<Json<Cart>> {
    let cart_id = parse_cart_id(&cart_id)?;
    let workflow = CartWorkflow::new(db.clone());
    let cart = workflow.get_cart(&cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    let context = cart_context(&db, &actor, &cart, query.country_code, query.province_code).await?;
    Ok(Json(
        workflow.remove_coupon(&cart_id, &code, &context).await?,
    ))
}

async fn create_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCheckoutRequestDto>,
) -> ApiResult<(StatusCode, Json<CheckoutSession>)> {
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
    let payment = PaymentService::with_registry(db.clone(), context.payment_providers.clone());
    let workflow = CheckoutWorkflow::new(db, payment);
    let mut checkout_request =
        CreateCheckoutRequest::new(cart_id, request.payment_provider, cart_context);
    checkout_request.email = request.email;
    checkout_request.idempotency_key = request.idempotency_key;
    if let Some(expires) = request.expires_in_seconds {
        checkout_request.expires_in_seconds = expires;
    }
    let checkout = workflow.create(checkout_request).await?;
    Ok((StatusCode::CREATED, Json(checkout)))
}

async fn get_checkout(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<CheckoutSession>> {
    let service = checkout_service(db.clone());
    let checkout = service
        .get(&fastcart_kernel::CheckoutId::from_string(checkout_id))
        .await?;
    let cart = CartWorkflow::new(db).get_cart(&checkout.cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(checkout))
}

async fn complete_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
    ApiJson(request): ApiJson<CompleteCheckoutRequest>,
) -> ApiResult<Json<CompleteCheckoutResponse>> {
    let checkout_id = fastcart_kernel::CheckoutId::from_string(checkout_id);
    let payment = PaymentService::with_registry(db.clone(), context.payment_providers.clone());
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
    Ok(Json(CompleteCheckoutResponse {
        checkout: result.checkout,
        order: result.order,
        payment: result.payment,
    }))
}

async fn cancel_checkout(
    State(context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<CheckoutSession>> {
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
        let payment = PaymentService::with_registry(db.clone(), context.payment_providers.clone());
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
    Ok(Json(checkout))
}

async fn checkout_history(
    State(_context): State<Arc<AppContext>>,
    Extension(actor): Extension<ActorContext>,
    StoreDb(db): StoreDb,
    Path(checkout_id): Path<String>,
) -> ApiResult<Json<Vec<fastcart_module_checkout::CheckoutStateHistory>>> {
    let checkout_id = fastcart_kernel::CheckoutId::from_string(checkout_id);
    let service = checkout_service(db.clone());
    let checkout = service.get(&checkout_id).await?;
    let cart = CartWorkflow::new(db).get_cart(&checkout.cart_id).await?;
    ensure_cart_access(&cart, &actor)?;
    Ok(Json(service.list_history(&checkout_id).await?))
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct DestinationQuery {
    /// Destination country.
    pub country_code: Option<String>,
    /// Destination province.
    pub province_code: Option<String>,
}

fn checkout_service(db: StoreDatabase) -> CheckoutService {
    CheckoutService::new(db)
}

fn parse_cart_id(value: &str) -> ApiResult<CartId> {
    if value.is_empty() || value.len() > 128 {
        return Err(ApiError::validation("invalid cart id"));
    }
    Ok(CartId::from_string(value))
}

async fn cart_context(
    db: &StoreDatabase,
    actor: &ActorContext,
    cart: &Cart,
    country_code: Option<String>,
    province_code: Option<String>,
) -> ApiResult<CartContext> {
    let country = country_code
        .or_else(|| cart.shipping_address.country_code.clone())
        .ok_or_else(|| {
            ApiError::validation(
                "a country_code is required (set a shipping address or pass country_code)",
            )
        })?;
    let country =
        CountryCode::new(country).map_err(|error| ApiError::new(AppError::from(error)))?;
    let mut customer_group_ids = Vec::new();
    if let Some(customer_id) = actor.customer_id.as_ref() {
        let groups = CustomerService::new(db.clone())
            .list_customer_groups(customer_id)
            .await?;
        customer_group_ids = groups.into_iter().map(|group| group.id).collect();
    }
    Ok(CartContext {
        country_code: country,
        province_code: province_code.or_else(|| cart.shipping_address.province.clone()),
        customer_group_ids,
        sales_channel_id: cart.sales_channel_id.clone(),
        at: now_timestamp(),
    })
}
