//! Domain entities and inputs for the theme registry.

use fastcart_kernel::{ThemeActivationId, ThemeId, Timestamp};
use fastcart_storefront::ThemeManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A registered storefront theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Internal theme identifier.
    pub id: ThemeId,
    /// Parsed and validated theme manifest.
    pub manifest: ThemeManifest,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// JSON Schema used to validate theme settings.
    pub settings_schema: Option<Value>,
    /// Current theme settings.
    pub settings: Value,
    /// Raw validated editor manifest, when the theme exposes an editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_manifest: Option<Value>,
    /// Editor manifest schema version; `1` for themes without an editor.
    pub schema_version: u32,
    /// Whether this theme is the store's active theme.
    pub is_active: bool,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for registering a theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterThemeInput {
    /// Theme manifest to parse and validate.
    pub manifest: ThemeManifest,
    /// Bundle-relative asset root.
    pub bundle_path: String,
    /// Optional JSON Schema for theme settings. Must be a JSON object.
    #[serde(default)]
    pub settings_schema: Option<Value>,
    /// Initial settings; defaults to an empty object.
    #[serde(default)]
    pub settings: Option<Value>,
    /// Raw editor manifest JSON. Required when the theme manifest references
    /// an editor and rejected otherwise.
    #[serde(default)]
    pub editor_manifest: Option<Value>,
}

/// Input for replacing theme settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateThemeSettingsInput {
    /// Replacement settings, validated against the theme's schema.
    pub settings: Value,
}

/// A recorded theme activation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeActivation {
    /// Activation identifier.
    pub id: ThemeActivationId,
    /// Theme that was activated.
    pub theme_id: ThemeId,
    /// Theme that was active before this activation, when any.
    pub previous_theme_id: Option<ThemeId>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}
