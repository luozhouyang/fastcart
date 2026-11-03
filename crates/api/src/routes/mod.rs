//! Shared route helpers and store-database middleware.

pub mod admin;
pub mod admin_catalog;
pub mod admin_content;
pub mod admin_customers;
pub mod admin_inventory;
pub mod admin_orders;
pub mod admin_payments;
pub mod admin_pricing;
pub mod admin_promotions;
pub mod admin_shipping;
pub mod admin_tax;
pub mod admin_theme;
pub mod admin_webhooks;
pub mod auth;
pub mod health;
pub mod platform;
pub mod store;
pub mod store_cart;
pub mod store_catalog;
pub mod store_content;
pub mod store_customer;
pub mod store_inventory;
pub mod store_theme;
pub mod storefront;
pub mod webhooks;

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, OriginalUri, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use fastcart_http::{
    ApiError, ApiResult, CSRF_HEADER, CUSTOMER_CSRF_COOKIE, CUSTOMER_SESSION_COOKIE, parse_cookies,
};
use fastcart_infrastructure::{StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{ActorContext, AppError, StoreContext, StoreId};
use fastcart_module_cart::Cart;
use fastcart_module_customer::CustomerService;
use fastcart_platform::{MembershipStatus, NewAudit, PlatformStore};
use subtle::ConstantTimeEq;

use crate::state::AppContext;

/// Appends an auditable store-scoped admin action.
///
/// Auditing is best-effort: a logging failure must not fail the business
/// operation that already completed.
pub async fn record_store_audit(
    context: &AppContext,
    actor: &ActorContext,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    data: serde_json::Value,
) {
    let entry = NewAudit {
        actor_id: actor.user_id.as_ref().map(ToString::to_string),
        store_id: actor.store_id.as_ref().map(ToString::to_string),
        action: action.to_owned(),
        resource_type: Some(resource_type.to_owned()),
        resource_id: Some(resource_id.to_owned()),
        data,
        ip_address: None,
    };
    if let Err(error) = context.platform.record_audit(entry).await {
        tracing::warn!(error = %error, action, "failed to write store audit log");
    }
}

/// Requires the configured secret on the internal storefront HTTP surface.
///
/// The in-process bridge does not need this middleware; it exists only for a
/// trusted sidecar that must call commerce services over loopback HTTP.
pub async fn internal_fetch_auth_middleware(
    State(expected): State<String>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let provided = request
        .headers()
        .get("x-fastcart-internal-secret")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !bool::from(provided.as_bytes().ct_eq(expected.as_bytes())) {
        return Err(ApiError::new(AppError::Forbidden));
    }
    Ok(next.run(request).await)
}

/// State for the store database middleware.
#[derive(Clone)]
pub struct StoreDbState {
    /// Manager providing isolated per-store pools.
    pub databases: Arc<StoreDatabaseManager>,
}

/// Loads the resolved store's database and attaches it to the request.
pub async fn store_database_middleware(
    State(state): State<StoreDbState>,
    Extension(store): Extension<StoreContext>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let database = state
        .databases
        .database(&store.store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    request.extensions_mut().insert(database);
    Ok(next.run(request).await)
}

/// Loads the admin-selected store's database for store-scoped admin routes.
pub async fn actor_store_database_middleware(
    State(state): State<StoreDbState>,
    Extension(actor): Extension<ActorContext>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let store_id = actor
        .store_id
        .as_ref()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let database = state
        .databases
        .database(store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?;
    request.extensions_mut().insert(database);
    Ok(next.run(request).await)
}

/// Extracts the authenticated store database from request extensions.
#[derive(Clone, Debug)]
pub struct StoreDb(pub StoreDatabase);

impl axum::extract::FromRequestParts<Arc<AppContext>> for StoreDb {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &Arc<AppContext>,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<StoreDatabase>()
            .cloned()
            .map(StoreDb)
            .ok_or_else(|| {
                ApiError::new(AppError::Internal(
                    "store database is missing from the request".to_owned(),
                ))
            })
    }
}

/// Verifies store membership and optionally a permission.
pub async fn authorize_store(
    platform: &PlatformStore,
    actor: &ActorContext,
    store_id: &StoreId,
    permission: Option<&str>,
) -> ApiResult<BTreeSet<String>> {
    let user_id = actor
        .user_id
        .as_ref()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let member = platform.is_member(user_id, store_id).await?;
    let permissions = platform.permissions_for(user_id, store_id).await?;
    if !member && permissions.is_empty() {
        return Err(ApiError::new(AppError::Forbidden));
    }
    if let Some(permission) = permission {
        if !permissions.contains(permission) {
            return Err(ApiError::new(AppError::Forbidden));
        }
    }
    Ok(permissions)
}

/// Requires the actor to hold the `owner` role for a store.
pub async fn require_owner(
    platform: &PlatformStore,
    actor: &ActorContext,
    store_id: &StoreId,
) -> ApiResult<()> {
    let user_id = actor
        .user_id
        .as_ref()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let membership = platform
        .get_membership(user_id, store_id)
        .await?
        .ok_or_else(|| ApiError::new(AppError::Forbidden))?;
    if membership.status == MembershipStatus::Active
        && membership.roles.iter().any(|role| role == "owner")
    {
        Ok(())
    } else {
        Err(ApiError::new(AppError::Forbidden))
    }
}

/// Fallback for routes that are not implemented yet.
pub async fn not_implemented(OriginalUri(uri): OriginalUri) -> ApiError {
    ApiError::new(AppError::NotImplemented(format!(
        "route {} is not implemented yet",
        uri.path()
    )))
}

/// JSON 501 fallback used by nested routers.
pub async fn not_implemented_json(OriginalUri(uri): OriginalUri) -> Json<serde_json::Value> {
    let _ = uri;
    Json(serde_json::json!({"error": {"code": "NOT_IMPLEMENTED"}}))
}

/// Authenticates a storefront customer from the session cookie.
///
/// Requests without a cookie continue anonymously. A presented but invalid
/// session is rejected with 401 so clients can clear stale cookies. Unsafe
/// cookie-authenticated requests require the double-submit CSRF token.
pub async fn customer_auth_middleware(
    Extension(database): Extension<StoreDatabase>,
    Extension(store): Extension<StoreContext>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let cookies = parse_cookies(request.headers());
    let actor = match cookies.get(CUSTOMER_SESSION_COOKIE) {
        Some(token) => {
            if !matches!(
                *request.method(),
                axum::http::Method::GET
                    | axum::http::Method::HEAD
                    | axum::http::Method::OPTIONS
                    | axum::http::Method::TRACE
            ) {
                let csrf_cookie = cookies
                    .get(CUSTOMER_CSRF_COOKIE)
                    .ok_or_else(|| ApiError::new(AppError::Forbidden))?;
                let csrf_header = request
                    .headers()
                    .get(CSRF_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| ApiError::new(AppError::Forbidden))?;
                if !bool::from(csrf_cookie.as_bytes().ct_eq(csrf_header.as_bytes())) {
                    return Err(ApiError::new(AppError::Forbidden));
                }
            }
            let service = CustomerService::new(database);
            let session = service
                .authenticate_session(token)
                .await
                .map_err(ApiError::new)?;
            ActorContext::customer(session.customer.id, store.store_id)
        }
        None => ActorContext {
            store_id: Some(store.store_id),
            ..ActorContext::anonymous()
        },
    };
    request.extensions_mut().insert(actor);
    Ok(next.run(request).await)
}

/// Ensures the actor may access a cart.
///
/// Anonymous carts are addressed by their unguessable identifier; carts that
/// belong to a customer require that customer's session. Cart identifiers are
/// never treated as authorization by themselves for customer carts.
pub fn ensure_cart_access(cart: &Cart, actor: &ActorContext) -> ApiResult<()> {
    match cart.customer_id.as_ref() {
        Some(customer_id) => {
            if actor.customer_id.as_ref() == Some(customer_id) {
                Ok(())
            } else {
                Err(ApiError::new(AppError::Forbidden))
            }
        }
        None => Ok(()),
    }
}
