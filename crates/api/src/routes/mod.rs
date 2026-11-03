//! Shared route helpers and store-database middleware.

pub mod admin;
pub mod auth;
pub mod health;
pub mod platform;
pub mod store;
pub mod webhooks;

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, OriginalUri, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use fastcart_http::{ApiError, ApiResult};
use fastcart_infrastructure::{StoreDatabase, StoreDatabaseManager};
use fastcart_kernel::{ActorContext, AppError, StoreContext, StoreId};
use fastcart_platform::{MembershipStatus, PlatformStore};

use crate::state::AppContext;

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
