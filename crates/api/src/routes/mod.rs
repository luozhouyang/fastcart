//! Shared route helpers and store-database middleware.

pub mod admin;
pub mod admin_assets;
pub mod admin_campaigns;
pub mod admin_catalog;
pub mod admin_content;
pub mod admin_customers;
pub mod admin_data_sources;
pub mod admin_inquiry;
pub mod admin_inventory;
pub mod admin_notifications;
pub mod admin_orders;
pub mod admin_payments;
pub mod admin_pricing;
pub mod admin_promotions;
pub mod admin_regions;
pub mod admin_shipping;
pub mod admin_tax;
pub mod admin_theme;
pub mod admin_webhooks;
pub mod auth;
pub mod health;
pub mod platform;
pub mod preview;
pub mod store;
pub mod store_assets;
pub mod store_cart;
pub mod store_catalog;
pub mod store_content;
pub mod store_context;
pub mod store_customer;
pub mod store_inquiry;
pub mod store_inventory;
pub mod store_regions;
pub mod store_theme;
pub mod storefront;
pub mod universal_assets;
pub mod universal_deployments;
pub mod webhooks;

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, OriginalUri, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use fastcart_http::{
    ApiError, ApiResult, CSRF_HEADER, CUSTOMER_CSRF_COOKIE, CUSTOMER_SESSION_COOKIE, parse_cookies,
};
use fastcart_infrastructure::{
    NewAuditOutbox, StoreDatabase, StoreDatabaseManager, append_audit_outbox, mark_audit_processed,
};
use fastcart_kernel::{ActorContext, AppError, StoreContext, StoreId};
use fastcart_module_cart::Cart;
use fastcart_module_customer::CustomerService;
use fastcart_platform::{MembershipStatus, NewAudit, PlatformStore};
use subtle::ConstantTimeEq;

use crate::state::AppContext;

/// Audits successful Admin mutations that have no service-level audit yet.
///
/// The service layer owns semantic audit entries for catalog, content, orders,
/// promotions and the theme. Customers, payments, pricing, shipping, tax and
/// inventory previously had no coverage, so this route-level fallback records
/// the method, path and status for every successful mutation in those
/// namespaces. It runs after authentication (the actor is in the request
/// extensions) and only records 2xx responses, so failed writes are not
/// mistaken for changes.
pub async fn admin_audit_middleware(
    State(context): State<Arc<AppContext>>,
    OriginalUri(original_uri): OriginalUri,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = original_uri.path().to_owned();
    let should_audit = matches!(
        method,
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    ) && is_audit_fallback_path(&path);
    let actor = request.extensions().get::<ActorContext>().cloned();
    let response = next.run(request).await;

    if should_audit
        && response.status().is_success()
        && let Some(actor) = actor
    {
        let status = response.status().as_u16();
        if let Some(store_id) = &actor.store_id
            && let Some(runtime) = &context.universal_storefront
        {
            runtime.invalidate_cache(store_id.as_str()).await;
        }
        record_store_audit(
            &context,
            &actor,
            &format!("admin.{}", method.as_str().to_ascii_lowercase()),
            "admin_route",
            &path,
            serde_json::json!({ "method": method.as_str(), "path": path, "status": status }),
        )
        .await;
    }

    response
}

fn is_audit_fallback_path(path: &str) -> bool {
    // `calculate` is a POST used for reads; it does not change state.
    if path.ends_with("/calculate") {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "/api/v1/admin/customers",
        "/api/v1/admin/payments",
        "/api/v1/admin/prices",
        "/api/v1/admin/price-lists",
        "/api/v1/admin/shipping",
        "/api/v1/admin/tax",
        "/api/v1/admin/inventory",
    ];
    PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

/// Appends an auditable store-scoped admin action.
///
/// Store-scoped actions are first written to the store's durable `audit_outbox`
/// and then delivered to the platform audit log immediately. If delivery or the
/// outbox write fails, the business operation still succeeds and maintenance
/// retries any row left pending. Platform-scope actions (no store id) are
/// written directly. Audit payloads are never logged.
pub async fn record_store_audit(
    context: &AppContext,
    actor: &ActorContext,
    action: &str,
    resource_type: &str,
    resource_id: &str,
    data: serde_json::Value,
) {
    let actor_id = actor.user_id.as_ref().map(ToString::to_string);

    let Some(store_id) = actor.store_id.clone() else {
        let entry = NewAudit {
            actor_id,
            store_id: None,
            action: action.to_owned(),
            resource_type: Some(resource_type.to_owned()),
            resource_id: Some(resource_id.to_owned()),
            data,
            ip_address: None,
        };
        if let Err(error) = context.platform.record_audit(entry).await {
            tracing::warn!(error = %error, action, "failed to write store audit log");
        }
        return;
    };

    // Resolve the store pool once; a failed open leaves no audit row, matching
    // the previous best-effort contract for an unreachable store.
    let database = match context.databases.database(&store_id).await {
        Ok(database) => database,
        Err(error) => {
            tracing::warn!(error = %error, action, "failed to open store database for audit log");
            return;
        }
    };

    let outbox_entry = NewAuditOutbox {
        actor_id: actor_id.clone(),
        store_id: Some(store_id.to_string()),
        action: action.to_owned(),
        resource_type: Some(resource_type.to_owned()),
        resource_id: Some(resource_id.to_owned()),
        data: data.clone(),
    };
    let outbox_id = match append_audit_outbox(database.pool(), &outbox_entry).await {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(error = %error, action, "failed to persist store audit log");
            return;
        }
    };

    let entry = NewAudit {
        actor_id,
        store_id: Some(store_id.to_string()),
        action: action.to_owned(),
        resource_type: Some(resource_type.to_owned()),
        resource_id: Some(resource_id.to_owned()),
        data,
        ip_address: None,
    };
    match context
        .platform
        .record_audit_with_id(&outbox_id, &entry)
        .await
    {
        Ok(()) => {
            if let Err(error) = mark_audit_processed(database.pool(), &outbox_id).await {
                tracing::warn!(error = %error, action, "failed to mark store audit delivered");
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                action,
                "failed to deliver store audit; queued for maintenance retry"
            );
        }
    }
}

/// Invalidates short-lived storefront context caches after successful mutations.
///
/// The customer and cart context caches are also bounded by a two-second TTL.
/// This middleware closes the window for the ordinary storefront flows (cart
/// edits, checkout, login, logout, profile updates) so a response that changes
/// the viewer's context is not followed by a stale render.
pub async fn storefront_context_invalidation_middleware(
    State(context): State<Arc<AppContext>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let store_id = request
        .extensions()
        .get::<StoreContext>()
        .map(|store| store.store_id.clone());
    let response = next.run(request).await;
    let cacheable = !matches!(
        method,
        axum::http::Method::GET
            | axum::http::Method::HEAD
            | axum::http::Method::OPTIONS
            | axum::http::Method::TRACE
    ) && response.status().is_success();
    if cacheable {
        // The store router is nested at `/api/v1/store`, so middleware inside
        // it sees paths relative to that prefix, while the upload router is
        // merged at the root with its full path. Accept both shapes.
        let under = |suffix: &str| {
            path.starts_with(suffix) || path.starts_with(&format!("/api/v1/store{suffix}"))
        };
        if under("/carts") || under("/checkouts") {
            context.invalidate_storefront_carts();
        }
        if under("/customers") {
            context.invalidate_storefront_customers();
            // Login, logout and profile changes can alter cart ownership.
            context.invalidate_storefront_carts();
        }
        if let Some(store_id) = &store_id
            && let Some(runtime) = &context.universal_storefront
        {
            runtime.invalidate_cache(store_id.as_str()).await;
        }
    }
    response
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

/// Store scope attached by the internal SSR credential middleware.
#[derive(Clone, Debug)]
pub struct InternalSsrScope {
    /// Allowed store; `None` means all stores.
    pub store_id: Option<String>,
}

/// Authenticates the server-only SSR commerce API.
pub async fn internal_ssr_auth_middleware(
    State(credentials): State<crate::state::InternalSsrCredentials>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let provided = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            request
                .headers()
                .get("x-fastcart-internal-token")
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or_default();
    let Some(credential) = credentials.validate(provided) else {
        return Err(ApiError::new(AppError::Forbidden));
    };
    request.extensions_mut().insert(InternalSsrScope {
        store_id: credential.store_id.clone(),
    });
    Ok(next.run(request).await)
}

/// State for the store database middleware.
#[derive(Clone)]
pub struct StoreDbState {
    /// Manager providing isolated per-store pools.
    pub databases: Arc<StoreDatabaseManager>,
}

/// Validated publishable API key context for storefront requests.
#[derive(Clone, Debug)]
pub struct PublishableKeyContext {
    /// Sales channels the key may access.
    pub sales_channel_ids: Vec<String>,
}

/// Validates the optional `x-publishable-api-key` storefront header.
pub async fn publishable_key_middleware(
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = request
        .headers()
        .get("x-publishable-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let Some(token) = token else {
        return Ok(next.run(request).await);
    };
    let database = request
        .extensions()
        .get::<StoreDatabase>()
        .cloned()
        .ok_or_else(|| ApiError::new(AppError::Unauthorized))?;
    let key = fastcart_module_store::RegionService::new(database)
        .authenticate_publishable_key(&token)
        .await?;
    let mut request = request;
    request.extensions_mut().insert(PublishableKeyContext {
        sales_channel_ids: key.sales_channel_ids,
    });
    Ok(next.run(request).await)
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
    if let Some(permission) = permission
        && !permissions.contains(permission)
    {
        return Err(ApiError::new(AppError::Forbidden));
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
