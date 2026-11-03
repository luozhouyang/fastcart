//! Admin API: authentication, store selection and store-scoped operations.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::middleware;
use axum::routing::{get, post};
use fastcart_http::{AdminActor, AdminAuthState, ApiError, ApiResult, PaginationQuery};

use super::auth::{login, logout, me};
use super::{StoreDbState, actor_store_database_middleware, not_implemented};
use crate::dto::{PageResponse, StoreDto};
use crate::state::AppContext;

/// Builds the admin router.
pub fn router(context: &Arc<AppContext>) -> Router<Arc<AppContext>> {
    let public = Router::new().route("/auth/login", post(login));

    let authenticated = Router::new()
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/stores", get(list_my_stores))
        .route_layer(middleware::from_fn_with_state(
            AdminAuthState {
                auth: context.auth.clone(),
                require_store: false,
            },
            fastcart_http::admin_auth_middleware,
        ));

    let scoped = Router::new()
        .merge(super::admin_catalog::router())
        .merge(super::admin_content::router())
        .merge(super::admin_customers::router())
        .merge(super::admin_inventory::router())
        .merge(super::admin_orders::router())
        .merge(super::admin_payments::router())
        .merge(super::admin_pricing::router())
        .merge(super::admin_promotions::router())
        .merge(super::admin_shipping::router())
        .merge(super::admin_tax::router())
        .merge(super::admin_theme::router())
        .merge(super::admin_webhooks::router())
        .fallback(not_implemented)
        .layer(middleware::from_fn_with_state(
            StoreDbState {
                databases: context.databases.clone(),
            },
            actor_store_database_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            AdminAuthState {
                auth: context.auth.clone(),
                require_store: true,
            },
            fastcart_http::admin_auth_middleware,
        ));

    public.merge(authenticated).merge(scoped)
}

async fn list_my_stores(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<StoreDto>>> {
    let user_id = actor.require_user().map_err(ApiError::new)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = context
        .platform
        .list_stores_for_user(user_id, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(
        page.map(|store| StoreDto::from(&store)),
    )))
}
