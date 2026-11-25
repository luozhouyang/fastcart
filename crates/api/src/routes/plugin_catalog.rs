//! Cached view of the enabled plugins for one store.
//!
//! Parsing a manifest deserializes the whole declaration (contributions, jobs,
//! settings schema). A storefront render used it twice per request — once to
//! resolve capability grants and once to inject storefront extensions — and
//! the admin catalog pages re-parsed every manifest on each load.
//!
//! The enabled set changes only through the plugin lifecycle routes and
//! capability binding routes, both of which bump the store cache generation,
//! so every derivation here stays valid until the next mutation.

use std::sync::Arc;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{AppError, StoreId};
use fastcart_plugin::{ContributionRegistry, PluginManifest};
use serde::{Deserialize, Serialize};

use crate::state::AppContext;

const CATALOG_NAMESPACE: &str = "plugin-catalog";
const CATALOG_KEY: &str = "enabled";

/// One enabled plugin installation with its parsed manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogPlugin {
    /// Plugin identifier.
    pub plugin_id: String,
    /// Installed artifact digest.
    pub digest: String,
    /// Plugin version string.
    pub version: String,
    /// Serialized settings object.
    pub settings_json: String,
    /// Parsed manifest.
    pub manifest: PluginManifest,
}

/// Enabled plugins for one store, ordered by plugin id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginCatalog {
    /// Enabled plugins, ordered by plugin id.
    pub plugins: Vec<CatalogPlugin>,
}

impl PluginCatalog {
    /// Returns one enabled plugin by id.
    pub fn plugin(&self, plugin_id: &str) -> Option<&CatalogPlugin> {
        self.plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
    }

    /// Builds a contribution registry over the enabled manifests.
    pub fn registry(&self) -> ContributionRegistry {
        ContributionRegistry::from_borrowed_plugins(
            self.plugins
                .iter()
                .map(|plugin| (plugin.plugin_id.as_str(), &plugin.manifest)),
        )
    }
}

/// Loads the enabled catalog, computing and caching it on a miss.
///
/// The cache key embeds the store generation, so a mutation that bumps the
/// generation simply stops matching; entries for older generations age out. On
/// a hit the parsed manifests are shared directly, without re-deserializing.
pub async fn load(
    context: &AppContext,
    database: &StoreDatabase,
) -> Result<Arc<PluginCatalog>, AppError> {
    let store_id = database.store_id();
    let key = context.cache.key(store_id, CATALOG_NAMESPACE, CATALOG_KEY);
    // `try_get_with` collapses concurrent misses into one build.
    let catalog = context
        .plugin_catalogs
        .try_get_with(key, async { build(database).await.map(Arc::new) })
        .await
        .map_err(|error| AppError::database(error.to_string()))?;
    Ok(catalog)
}

async fn build(database: &StoreDatabase) -> Result<PluginCatalog, AppError> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT plugin_id, digest, version, settings_json, manifest_json \
         FROM plugin_installations WHERE enabled = 1 ORDER BY plugin_id ASC",
    )
    .fetch_all(database.pool())
    .await
    .map_err(database_error)?;

    let mut plugins = Vec::with_capacity(rows.len());
    for (plugin_id, digest, version, settings_json, manifest_json) in rows {
        let Ok(manifest) = PluginManifest::parse(manifest_json.as_bytes()) else {
            tracing::warn!(
                plugin_id,
                store_id = database.store_id().as_str(),
                "skipping enabled plugin with an invalid manifest"
            );
            continue;
        };
        plugins.push(CatalogPlugin {
            plugin_id,
            digest,
            version,
            settings_json,
            manifest,
        });
    }
    Ok(PluginCatalog { plugins })
}

/// Drops every cached derivation for a store after a plugin or binding change.
///
/// Bumps the store cache generation and invalidates cached SSR renders, which
/// embed the resolved grants.
pub(crate) async fn invalidate(context: &AppContext, store_id: &StoreId) {
    context.cache.invalidate_store(store_id);
    super::invalidate_storefront_cache(context, store_id.as_str()).await;
}
