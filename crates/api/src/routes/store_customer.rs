//! Public storefront customer routes.
//!
//! Session cookies are scoped to the resolved store. Handlers never read a
//! customer identifier from the client: profile, address and order access always
//! uses the authenticated actor's identifier.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header::{HeaderValue, SET_COOKIE};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use fastcart_http::{
    ApiError, ApiJson, ApiResult, CUSTOMER_CSRF_COOKIE, CUSTOMER_SESSION_COOKIE, CustomerActor,
    PaginationQuery, csrf_cookie, parse_cookies, session_cookie, session_cookie_clear,
};
use fastcart_kernel::{AppError, OrderId, now_timestamp};
use fastcart_module_customer::{
    Address, AddressInput, CreatedCustomerSession, Customer, CustomerGroup, CustomerService,
    RegisterCustomerInput, UpdateCustomerInput,
};
use fastcart_module_order::{Order, OrderFilter, OrderService};
use serde::{Deserialize, Serialize};

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the public customer routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/customers", post(register))
        .route("/customers/login", post(login))
        .route("/customers/logout", post(logout))
        .route("/customers/me", get(me).patch(update_me))
        .route(
            "/customers/me/addresses",
            get(list_addresses).post(add_address),
        )
        .route(
            "/customers/me/addresses/{address_id}",
            axum::routing::patch(update_address).delete(remove_address),
        )
        .route("/customers/me/orders", get(list_orders))
        .route("/customers/me/orders/{order_id}", get(get_own_order))
        .route("/customers/me/groups", get(list_groups))
        .route("/customers/me/password", post(change_password))
}

/// Registration request body.
#[derive(Clone, Debug, Deserialize)]
struct RegisterRequest {
    /// Account email.
    email: String,
    /// Plaintext password.
    password: String,
    /// Given name.
    #[serde(default)]
    first_name: Option<String>,
    /// Family name.
    #[serde(default)]
    last_name: Option<String>,
}

/// Customer login request body.
#[derive(Clone, Debug, Deserialize)]
struct CustomerLoginRequest {
    /// Account email.
    email: String,
    /// Plaintext password.
    password: String,
}

/// Profile update request body. Omitted fields are left unchanged.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct UpdateProfileRequest {
    /// Replacement given name.
    first_name: Option<String>,
    /// Replacement family name.
    last_name: Option<String>,
    /// Replacement phone number.
    phone: Option<String>,
}

/// Address create/update request body. Missing fields fall back to the existing
/// address values when updating.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct AddressRequest {
    /// Default shipping flag.
    is_default_shipping: Option<bool>,
    /// Default billing flag.
    is_default_billing: Option<bool>,
    /// Recipient given name.
    first_name: Option<String>,
    /// Recipient family name.
    last_name: Option<String>,
    /// Company name.
    company: Option<String>,
    /// First address line.
    address_1: Option<String>,
    /// Second address line.
    address_2: Option<String>,
    /// City or locality.
    city: Option<String>,
    /// Province, region or state.
    province: Option<String>,
    /// Postal or ZIP code.
    postal_code: Option<String>,
    /// Two-letter country code.
    country_code: Option<String>,
    /// Contact phone number.
    phone: Option<String>,
}

/// Password change request body.
#[derive(Clone, Debug, Deserialize)]
struct ChangePasswordRequest {
    /// Current password.
    current_password: String,
    /// Replacement password.
    new_password: String,
}

/// Session response body. Tokens are delivered as cookies.
#[derive(Clone, Debug, Serialize)]
struct CustomerSessionResponse {
    /// Authenticated customer.
    customer: Customer,
    /// Double-submit CSRF token.
    csrf_token: String,
    /// Session expiry timestamp.
    expires_at: i64,
}

async fn register(
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

async fn login(
    State(context): State<Arc<AppContext>>,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CustomerLoginRequest>,
) -> ApiResult<Response> {
    let session = CustomerService::new(db)
        .login(&request.email, &request.password)
        .await?;
    session_response(&context, session)
}

async fn logout(
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

async fn me(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<Customer>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let customer = CustomerService::new(db).get_customer(&customer_id).await?;
    Ok(Json(customer))
}

async fn update_me(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    ApiJson(request): ApiJson<UpdateProfileRequest>,
) -> ApiResult<Json<Customer>> {
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
    Ok(Json(customer))
}

async fn list_addresses(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<Vec<Address>>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let addresses = CustomerService::new(db)
        .list_addresses(&customer_id)
        .await?;
    Ok(Json(addresses))
}

async fn add_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    ApiJson(request): ApiJson<AddressRequest>,
) -> ApiResult<(StatusCode, Json<Address>)> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let address = CustomerService::new(db)
        .add_address(&customer_id, address_input(request, None))
        .await?;
    Ok((StatusCode::CREATED, Json(address)))
}

async fn update_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(address_id): Path<String>,
    ApiJson(request): ApiJson<AddressRequest>,
) -> ApiResult<Json<Address>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let service = CustomerService::new(db);
    let existing = service.get_address(&customer_id, &address_id).await?;
    let address = service
        .update_address(
            &customer_id,
            &address_id,
            address_input(request, Some(&existing)),
        )
        .await?;
    Ok(Json(address))
}

async fn remove_address(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    Path(address_id): Path<String>,
) -> ApiResult<StatusCode> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    CustomerService::new(db)
        .remove_address(&customer_id, &address_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_orders(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Order>>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let filter = OrderFilter {
        customer_id: Some(customer_id),
        ..OrderFilter::default()
    };
    let page = OrderService::new(db).list(filter, pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn list_groups(
    StoreDb(db): StoreDb,
    CustomerActor(actor): CustomerActor,
) -> ApiResult<Json<Vec<CustomerGroup>>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?.clone();
    let groups = CustomerService::new(db)
        .list_customer_groups(&customer_id)
        .await?;
    Ok(Json(groups))
}

async fn change_password(
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

/// Builds the login response, setting the session and CSRF cookies.
fn session_response(context: &AppContext, session: CreatedCustomerSession) -> ApiResult<Response> {
    let max_age = session.expires_at.saturating_sub(now_timestamp());
    let body = Json(CustomerSessionResponse {
        customer: session.customer,
        csrf_token: session.csrf_token.clone(),
        expires_at: session.expires_at,
    });
    let mut response = body.into_response();
    append_cookie(
        &mut response,
        session_cookie(
            CUSTOMER_SESSION_COOKIE,
            &session.token,
            max_age,
            context.config.cookie_secure,
        ),
    )?;
    append_cookie(
        &mut response,
        csrf_cookie(
            CUSTOMER_CSRF_COOKIE,
            &session.csrf_token,
            context.config.cookie_secure,
        ),
    )?;
    Ok(response)
}

/// Clears the customer session and CSRF cookies.
fn clear_session_cookies(context: &AppContext, response: &mut Response) -> ApiResult<()> {
    append_cookie(
        response,
        session_cookie_clear(CUSTOMER_SESSION_COOKIE, context.config.cookie_secure),
    )?;
    append_cookie(
        response,
        session_cookie_clear(CUSTOMER_CSRF_COOKIE, context.config.cookie_secure),
    )
}

fn append_cookie(response: &mut Response, cookie: String) -> ApiResult<()> {
    let value = HeaderValue::from_str(&cookie)
        .map_err(|_| ApiError::new(AppError::Internal("invalid cookie".to_owned())))?;
    response.headers_mut().append(SET_COOKIE, value);
    Ok(())
}

/// Merges an address request with the existing address, when updating.
fn address_input(request: AddressRequest, existing: Option<&Address>) -> AddressInput {
    let is_default_shipping = match request.is_default_shipping {
        Some(value) => value,
        None => existing.is_some_and(|address| address.is_default_shipping),
    };
    let is_default_billing = match request.is_default_billing {
        Some(value) => value,
        None => existing.is_some_and(|address| address.is_default_billing),
    };
    AddressInput {
        is_default_shipping,
        is_default_billing,
        first_name: request
            .first_name
            .or_else(|| existing.and_then(|address| address.first_name.clone())),
        last_name: request
            .last_name
            .or_else(|| existing.and_then(|address| address.last_name.clone())),
        company: request
            .company
            .or_else(|| existing.and_then(|address| address.company.clone())),
        address_1: request
            .address_1
            .or_else(|| existing.and_then(|address| address.address_1.clone())),
        address_2: request
            .address_2
            .or_else(|| existing.and_then(|address| address.address_2.clone())),
        city: request
            .city
            .or_else(|| existing.and_then(|address| address.city.clone())),
        province: request
            .province
            .or_else(|| existing.and_then(|address| address.province.clone())),
        postal_code: request
            .postal_code
            .or_else(|| existing.and_then(|address| address.postal_code.clone())),
        country_code: request.country_code.or_else(|| {
            existing.and_then(|address| {
                address
                    .country_code
                    .as_ref()
                    .map(|code| code.as_str().to_owned())
            })
        }),
        phone: request
            .phone
            .or_else(|| existing.and_then(|address| address.phone.clone())),
        metadata: existing.map_or_else(Default::default, |address| address.metadata.clone()),
    }
}

async fn get_own_order(
    CustomerActor(actor): CustomerActor,
    StoreDb(db): StoreDb,
    Path(order_id): Path<String>,
) -> ApiResult<Json<Order>> {
    let customer_id = actor.require_customer().map_err(ApiError::new)?;
    let order = OrderService::new(db)
        .get(&OrderId::from_string(order_id))
        .await?;
    if order.customer_id.as_ref() != Some(customer_id) {
        return Err(ApiError::new(AppError::not_found("order", &order.id)));
    }
    Ok(Json(order))
}

#[cfg(test)]
mod tests {
    use super::{AddressRequest, address_input};

    #[test]
    fn address_input_maps_request_fields() {
        let input = address_input(
            AddressRequest {
                is_default_shipping: Some(true),
                address_1: Some("1 Main St".to_owned()),
                city: Some("Springfield".to_owned()),
                postal_code: Some("12345".to_owned()),
                country_code: Some("US".to_owned()),
                ..AddressRequest::default()
            },
            None,
        );

        assert!(input.is_default_shipping);
        assert!(!input.is_default_billing);
        assert_eq!(input.address_1.as_deref(), Some("1 Main St"));
        assert_eq!(input.city.as_deref(), Some("Springfield"));
        assert_eq!(input.postal_code.as_deref(), Some("12345"));
        assert_eq!(input.country_code.as_deref(), Some("US"));
    }

    #[test]
    fn address_input_defaults_without_existing_address() {
        let input = address_input(AddressRequest::default(), None);
        assert!(!input.is_default_shipping);
        assert!(!input.is_default_billing);
        assert!(input.address_1.is_none());
        assert!(input.country_code.is_none());
    }
}
