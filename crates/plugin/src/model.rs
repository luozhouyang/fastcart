//! Public plugin installation models.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::manifest::PluginManifest;

/// An installed plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct InstalledPlugin {
    /// Plugin id.
    pub id: String,
    /// Display name from the manifest.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Active artifact digest.
    pub digest: String,
    /// Digest available for rollback.
    pub previous_digest: Option<String>,
    /// Whether the plugin is enabled for this store.
    pub enabled: bool,
    /// Published settings.
    #[schema(value_type = Object)]
    pub settings: Value,
    /// Monotonic settings revision.
    pub settings_revision: i64,
    /// Capabilities accepted at install time.
    pub granted_scopes: Vec<String>,
    /// Unix timestamp of installation.
    pub installed_at: i64,
    /// Unix timestamp of the last change.
    pub updated_at: i64,
}

/// Metadata for one stored plugin secret.
///
/// Values are never returned by the admin API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginSecretName {
    /// Secret name.
    pub name: String,
    /// Unix timestamp of the last update.
    pub updated_at: i64,
}

/// Installation plus the full manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginDetail {
    /// Installation record.
    pub plugin: InstalledPlugin,
    /// Validated manifest.
    #[schema(value_type = Object)]
    pub manifest: PluginManifest,
}
