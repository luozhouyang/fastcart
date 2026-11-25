//! Admin storefront component contributions.
//!
//! Components declared by enabled plugins are indexed from their manifests.
//! Entry modules are served by the public storefront extension route, so the
//! response only exposes generated storefront URLs.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::get;
use fastcart_http::{AdminActor, ApiError, ApiResult};
use fastcart_kernel::ActorContext;
use fastcart_plugin::Contribution;
use serde::Serialize;
use serde_json::Value;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the admin storefront component routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route("/components", get(list_components))
}

fn require(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

/// One capability required by a storefront component.
#[derive(Clone, Debug, Serialize)]
pub struct ComponentRequirementView {
    /// Required capability identifier.
    pub id: String,
    /// Required capability version; any version when omitted.
    pub version: Option<u32>,
    /// Whether the component works without the capability.
    pub optional: bool,
}

/// One storefront component contribution exposed to administrators.
#[derive(Clone, Debug, Serialize)]
pub struct ComponentContributionView {
    /// Providing plugin.
    pub plugin_id: String,
    /// Plugin display name.
    pub plugin_name: String,
    /// Plugin version.
    pub plugin_version: String,
    /// Installed artifact digest.
    pub digest: String,
    /// Component identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Optional editor category.
    pub category: Option<String>,
    /// Optional editor metadata for host tooling.
    pub editor_manifest: Option<Value>,
    /// Public storefront URL for the entry module.
    pub entry: Option<String>,
    /// Capabilities the component requires.
    pub requires: Vec<ComponentRequirementView>,
}

/// Storefront components declared by enabled plugins.
#[derive(Clone, Debug, Serialize)]
pub struct ComponentsResponse {
    /// Flat component list across enabled plugins.
    pub components: Vec<ComponentContributionView>,
}

async fn list_components(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ComponentsResponse>> {
    require(&actor, "plugins.read")?;
    let catalog = super::plugin_catalog::load(&context, &db)
        .await
        .map_err(ApiError::new)?;
    let mut components = Vec::new();
    for provider in catalog.registry().components() {
        let Contribution::StorefrontComponent(component) = &provider.contribution else {
            continue;
        };
        let Some(plugin) = catalog.plugin(&provider.plugin_id) else {
            continue;
        };
        let entry = component.entry.as_ref().map(|entry| {
            format!(
                "/_fastcart/extensions/{}/{}/{}",
                plugin.plugin_id,
                plugin.digest,
                entry.trim_start_matches('/'),
            )
        });
        components.push(ComponentContributionView {
            plugin_id: provider.plugin_id.clone(),
            plugin_name: provider.plugin_name.clone(),
            plugin_version: provider.plugin_version.clone(),
            digest: plugin.digest.clone(),
            id: component.id.clone(),
            title: component.title.clone(),
            category: component.category.clone(),
            editor_manifest: component.editor_manifest.clone(),
            entry,
            requires: component
                .requires
                .iter()
                .map(|requirement| ComponentRequirementView {
                    id: requirement.id.clone(),
                    version: requirement.version,
                    optional: requirement.optional,
                })
                .collect(),
        });
    }
    Ok(Json(ComponentsResponse { components }))
}
