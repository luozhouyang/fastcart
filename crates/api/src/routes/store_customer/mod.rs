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
use fastcart_email::EmailMessage;
use fastcart_http::{
    ApiError, ApiJson, ApiResult, CUSTOMER_CSRF_COOKIE, CUSTOMER_SESSION_COOKIE, CustomerActor,
    PaginationQuery, csrf_cookie, parse_cookies, session_cookie, session_cookie_clear,
};
use fastcart_infrastructure::{JobQueue, StoreDatabase};
use fastcart_kernel::{AppError, Metadata, OrderId, normalize_email, now_timestamp};
use fastcart_module_customer::{
    Address, AddressInput, CreatedCustomerSession, CustomerService, RegisterCustomerInput,
    UpdateCustomerInput,
};
use fastcart_module_order::{OrderFilter, OrderService};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{
    CustomerAddressListResponse, CustomerAddressResponse, CustomerDto, CustomerGroupsResponse,
    CustomerResponse, OrderListResponse, OrderResponse,
};
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
        .route(
            "/customers/email-verification/request",
            post(request_email_verification),
        )
        .route(
            "/customers/email-verification/confirm",
            post(confirm_email_verification),
        )
        .route(
            "/customers/password-reset/request",
            post(request_password_reset),
        )
        .route(
            "/customers/password-reset/confirm",
            post(confirm_password_reset),
        )
}

/// Builds the documented public customer routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(register))
        .routes(routes!(login))
        .routes(routes!(logout))
        .routes(routes!(me, update_me))
        .routes(routes!(list_addresses, add_address))
        .routes(routes!(update_address, remove_address))
        .routes(routes!(list_orders))
        .routes(routes!(get_own_order))
        .routes(routes!(list_groups))
        .routes(routes!(change_password))
        .routes(routes!(request_email_verification))
        .routes(routes!(confirm_email_verification))
        .routes(routes!(request_password_reset))
        .routes(routes!(confirm_password_reset))
}

mod addresses;
mod auth;
mod groups;
mod orders;
mod verification;

use addresses::*;
use auth::*;
use groups::*;
use orders::*;
use verification::*;

/// Registration request body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = RegisterCustomerRequest)]
pub struct RegisterRequest {
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
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CustomerLoginRequest {
    /// Account email.
    email: String,
    /// Plaintext password.
    password: String,
}

/// Profile update request body. Omitted fields are left unchanged.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateProfileRequest {
    /// Replacement given name.
    first_name: Option<String>,
    /// Replacement family name.
    last_name: Option<String>,
    /// Replacement phone number.
    phone: Option<String>,
}

/// Address create/update request body. Missing fields fall back to the existing
/// address values when updating.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct AddressRequest {
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
    /// Free-form extension metadata.
    metadata: Metadata,
}

/// Password change request body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ChangePasswordRequest {
    /// Current password.
    current_password: String,
    /// Replacement password.
    new_password: String,
}

/// Email-only request body used by the verification and reset request routes.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct EmailRequest {
    /// Account email address.
    email: String,
}

/// password-reset request body.
///
/// Shares the wire shape of [`EmailRequest`] but carries its own documented
/// schema name.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct PasswordResetRequest {
    /// Account email address.
    email: String,
}

/// Single-token request body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = EmailVerificationConfirmRequest)]
pub struct TokenRequest {
    /// Plaintext single-use token.
    token: String,
}

/// password-reset confirmation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[schema(as = PasswordResetConfirmRequest)]
pub struct ResetPasswordRequest {
    /// Plaintext single-use token.
    token: String,
    /// Replacement password.
    password: String,
}

/// Generic acknowledgement that never reveals account existence or state.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct AcceptedResponse {
    /// The request was accepted for processing.
    accepted: bool,
}

/// Confirmation result for an email-verification token.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct EmailVerifiedResponse {
    /// The customer's email is now verified.
    email_verified: bool,
}

/// Session response body. Tokens are delivered as cookies.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerSessionResponse {
    /// Authenticated customer.
    customer: CustomerDto,
    /// Double-submit CSRF token.
    csrf_token: String,
    /// Session expiry timestamp.
    expires_at: i64,
}

/// Enqueues an authentication email, logging failures without failing the
/// caller. Plaintext tokens are never logged.
async fn enqueue_auth_email(
    context: &AppContext,
    db: &StoreDatabase,
    recipient: &str,
    subject: &str,
    body: String,
) {
    let message = EmailMessage::new(recipient, subject).with_text(body);
    let queue = JobQueue::new(db.pool().clone());
    if let Err(error) = context.email.enqueue(&queue, message, Some(10)).await {
        tracing::warn!(
            error = %error,
            "failed to enqueue customer authentication email"
        );
    }
}

/// Builds the login response, setting the session and CSRF cookies.
fn session_response(context: &AppContext, session: CreatedCustomerSession) -> ApiResult<Response> {
    let max_age = session.expires_at.saturating_sub(now_timestamp());
    let body = Json(CustomerSessionResponse {
        customer: session.customer.into(),
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
            max_age,
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
