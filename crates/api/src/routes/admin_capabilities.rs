//! Admin capability catalog and store bindings.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use fastcart_http::{AdminActor, ApiError, ApiResult};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::ActorContext;
use fastcart_plugin::{CapabilityBinding, CapabilityBindingService, Contribution, PluginError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the admin capability routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/capabilities", get(list_capabilities))
        .route(
            "/capabilities/{id}/binding",
            put(bind_capability).delete(unbind_capability),
        )
}

fn require(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

fn api(error: PluginError) -> ApiError {
    ApiError::new(error.into())
}

fn service(db: StoreDatabase, actor: &ActorContext) -> CapabilityBindingService {
    CapabilityBindingService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
}

/// One capability contribution exposed to administrators.
#[derive(Clone, Debug, Serialize)]
pub struct CapabilityContributionView {
    /// Providing plugin.
    pub plugin_id: String,
    /// Plugin display name.
    pub plugin_name: String,
    /// Plugin version.
    pub plugin_version: String,
    /// Capability identifier.
    pub id: String,
    /// Capability version.
    pub version: u32,
    /// Human-readable title.
    pub title: String,
    /// Optional config schema.
    pub config_schema: Option<Value>,
}

/// Capability catalog and current bindings.
#[derive(Clone, Debug, Serialize)]
pub struct CapabilitiesResponse {
    /// Capabilities provided by enabled plugins.
    pub contributions: Vec<CapabilityContributionView>,
    /// Bindings configured for this store.
    pub bindings: Vec<CapabilityBinding>,
}

async fn list_capabilities(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<CapabilitiesResponse>> {
    require(&actor, "plugins.read")?;
    let catalog = super::plugin_catalog::load(&context, &db)
        .await
        .map_err(ApiError::new)?;
    let contributions = catalog
        .registry()
        .into_capabilities()
        .into_iter()
        .filter_map(|provider| match provider.contribution {
            Contribution::StorefrontCapability(capability) => Some(CapabilityContributionView {
                plugin_id: provider.plugin_id,
                plugin_name: provider.plugin_name,
                plugin_version: provider.plugin_version,
                id: capability.id,
                version: capability.version,
                title: capability.title,
                config_schema: capability.config_schema,
            }),
            Contribution::StorefrontComponent(_) => None,
        })
        .collect();
    let bindings = service(db, &actor).list().await.map_err(api)?;
    Ok(Json(CapabilitiesResponse {
        contributions,
        bindings,
    }))
}

/// Binding replacement payload.
#[derive(Clone, Debug, Deserialize)]
pub struct BindCapabilityRequest {
    /// Providing plugin.
    pub plugin_id: String,
    /// Provided capability version.
    pub version: u32,
    /// Store configuration for the capability.
    #[serde(default)]
    pub config: Option<Value>,
    /// Whether the route resolves the binding.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

async fn bind_capability(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    Json(request): Json<BindCapabilityRequest>,
) -> ApiResult<Json<CapabilityBinding>> {
    require(&actor, "plugins.write")?;
    let binding = service(db.clone(), &actor)
        .bind(
            &request.plugin_id,
            &id,
            request.version,
            request.config.unwrap_or_else(|| serde_json::json!({})),
            request.enabled,
        )
        .await
        .map_err(api)?;
    invalidate_storefront(&context, &db).await;
    Ok(Json(binding))
}

async fn unbind_capability(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    service(db.clone(), &actor).unbind(&id).await.map_err(api)?;
    invalidate_storefront(&context, &db).await;
    Ok(StatusCode::NO_CONTENT)
}

/// A binding change moves the catalog generation and cached renders with it.
async fn invalidate_storefront(context: &AppContext, db: &StoreDatabase) {
    super::plugin_catalog::invalidate(context, db.store_id()).await;
}
