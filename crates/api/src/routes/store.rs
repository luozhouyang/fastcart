//! Storefront and internal storefront routers.
//!
//! Every route is evaluated inside a trusted store context. The `Host` header is
//! resolved against the platform registry; clients cannot select a store with a
//! header or query parameter.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use fastcart_http::{ApiError, StoreResolutionState};
use fastcart_kernel::{AppError, CurrencyCode, StoreContext, StoreId};
use fastcart_platform::StoreStatus;

use super::{
    InternalSsrScope, StoreDbState, customer_auth_middleware, not_implemented,
    publishable_key_middleware, store_database_middleware,
};
use crate::state::AppContext;

/// Builds the public storefront router.
pub fn router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    scoped(context)
}

/// Builds the attachment upload router mounted with its own body limit.
///
/// The route carries its full path because it is merged into the root router
/// after the common middleware layers have been applied.
pub fn upload_router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    store_layers(
        context,
        Router::new()
            .route(
                "/api/v1/store/inquiry-files",
                axum::routing::post(super::store_inquiry::upload_file),
            )
            // `Multipart` uses Axum's extractor limit (2 MiB by default), which
            // is separate from the outer `RequestBodyLimitLayer`.
            .layer(axum::extract::DefaultBodyLimit::max(
                context.config.max_upload_bytes,
            )),
    )
}

/// Builds the legacy internal SSR-to-commerce router.
///
/// The internal namespace is an application boundary: it is not exposed on a
/// separate public listener and still resolves the trusted store context.
pub fn internal_router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    scoped(context)
}

/// Builds the server-only SSR commerce router.
///
/// Store selection comes from the nested `{store_id}` path, never from the
/// public `Host` header. The caller must already be authenticated with the
/// configured bearer token.
pub fn internal_ssr_router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    internal_store_layers(context, store_routes())
}

fn scoped(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    store_layers(context, store_routes())
}

fn store_routes() -> Router<Arc<AppContext>> {
    Router::new()
        .merge(super::store_assets::router())
        .merge(super::store_cart::router())
        .merge(super::store_catalog::router())
        .merge(super::store_customer::router())
        .merge(super::store_content::router())
        .merge(super::store_context::router())
        .merge(super::store_inventory::router())
        .merge(super::store_inquiry::router())
        .merge(super::store_theme::router())
        .merge(super::store_regions::router())
        .fallback(not_implemented)
}

fn store_layers(
    context: &Arc<AppContext>,
    router: Router<Arc<AppContext>>,
) -> Router<Arc<AppContext>> {
    router
        .layer(middleware::from_fn_with_state(
            context.clone(),
            super::storefront_context_invalidation_middleware,
        ))
        .layer(middleware::from_fn(publishable_key_middleware))
        .layer(middleware::from_fn(customer_auth_middleware))
        .layer(middleware::from_fn_with_state(
            StoreDbState {
                databases: context.databases.clone(),
            },
            store_database_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            StoreResolutionState {
                resolver: context.resolver.clone(),
            },
            fastcart_http::store_resolution_middleware,
        ))
}

/// State for resolving a store from an internal path parameter.
#[derive(Clone)]
pub struct InternalStoreState {
    /// Application context used for platform lookup.
    pub context: Arc<AppContext>,
}

impl std::fmt::Debug for InternalStoreState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("InternalStoreState(..)")
    }
}

/// Resolves the internal path store and installs a trusted store context.
pub async fn internal_store_middleware(
    State(state): State<InternalStoreState>,
    Path(store_id): Path<String>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let store_id = StoreId::from_string(store_id);
    let store = state
        .context
        .platform
        .get_store(&store_id)
        .await
        .map_err(|error| ApiError::new(error.into()))?
        .filter(|store| store.status == StoreStatus::Active)
        .ok_or_else(|| ApiError::new(AppError::not_found("store", store_id.as_str())))?;
    if let Some(scope) = request.extensions().get::<InternalSsrScope>()
        && scope
            .store_id
            .as_deref()
            .is_some_and(|allowed| allowed != store.id.as_str())
    {
        return Err(ApiError::new(AppError::Forbidden));
    }
    let currency = CurrencyCode::new(store.currency_code.clone())
        .map_err(|error| ApiError::new(AppError::from(error)))?;
    request.extensions_mut().insert(StoreContext {
        store_id: store.id,
        domain: store.primary_domain.unwrap_or_default(),
        locale: store.locale,
        currency,
        timezone: store.timezone,
    });
    Ok(next.run(request).await)
}

fn internal_store_layers(
    context: &Arc<AppContext>,
    router: Router<Arc<AppContext>>,
) -> Router<Arc<AppContext>> {
    router
        .layer(middleware::from_fn_with_state(
            context.clone(),
            super::storefront_context_invalidation_middleware,
        ))
        .layer(middleware::from_fn(publishable_key_middleware))
        .layer(middleware::from_fn(customer_auth_middleware))
        .layer(middleware::from_fn_with_state(
            StoreDbState {
                databases: context.databases.clone(),
            },
            store_database_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            InternalStoreState {
                context: context.clone(),
            },
            internal_store_middleware,
        ))
}
