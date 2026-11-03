//! Storefront and internal storefront routers.
//!
//! Every route is evaluated inside a trusted store context. The `Host` header is
//! resolved against the platform registry; clients cannot select a store with a
//! header or query parameter.

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use fastcart_http::StoreResolutionState;

use super::{
    StoreDbState, customer_auth_middleware, not_implemented, publishable_key_middleware,
    store_database_middleware,
};
use crate::state::AppContext;

/// Builds the public storefront router.
pub fn router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    scoped(context)
}

/// Builds the internal SSR-to-commerce router.
///
/// The internal namespace is an application boundary: it is not exposed on a
/// separate public listener and still resolves the trusted store context.
pub fn internal_router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    scoped(context)
}

fn scoped(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    Router::new()
        .merge(super::store_cart::router())
        .merge(super::store_catalog::router())
        .merge(super::store_customer::router())
        .merge(super::store_content::router())
        .merge(super::store_inventory::router())
        .merge(super::store_theme::router())
        .merge(super::store_regions::router())
        .fallback(not_implemented)
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
