//! Admin storefront component contributions.
//!
//! Components declared by enabled plugins are indexed from their manifests.
//! Entry modules are served by the public storefront extension route, so the
//! response only exposes generated storefront URLs.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::routing::get;
use fastcart_http::{AdminActor, ApiError, ApiResult};
use fastcart_kernel::ActorContext;
use fastcart_plugin::{Contribution, ContributionRegistry, PluginManifest};
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
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ComponentsResponse>> {
    require(&actor, "plugins.read")?;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT plugin_id, digest, manifest_json FROM plugin_installations \
         WHERE enabled = 1 ORDER BY plugin_id ASC",
    )
    .fetch_all(db.pool())
    .await
    .map_err(|error| ApiError::new(fastcart_kernel::AppError::database(error.to_string())))?;

    let mut plugins = Vec::with_capacity(rows.len());
    let mut digests = BTreeMap::new();
    for (plugin_id, digest, manifest_json) in rows {
        let Ok(manifest) = PluginManifest::parse(manifest_json.as_bytes()) else {
            continue;
        };
        digests.insert(plugin_id.clone(), digest);
        plugins.push((plugin_id, manifest));
    }

    let registry = ContributionRegistry::from_plugins(plugins);
    let components = registry
        .components()
        .into_iter()
        .filter_map(|provider| {
            let Contribution::StorefrontComponent(component) = &provider.contribution else {
                return None;
            };
            let digest = digests.get(&provider.plugin_id)?.clone();
            let entry = component.entry.as_ref().map(|entry| {
                format!(
                    "/_fastcart/extensions/{}/{}/{}",
                    provider.plugin_id,
                    digest,
                    entry.trim_start_matches('/'),
                )
            });
            Some(ComponentContributionView {
                plugin_id: provider.plugin_id.clone(),
                plugin_name: provider.plugin_name.clone(),
                plugin_version: provider.plugin_version.clone(),
                digest,
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
            })
        })
        .collect();
    Ok(Json(ComponentsResponse { components }))
}
