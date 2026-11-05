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
    /// Validated assets manifest loaded from the theme bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets_manifest: Option<Value>,
    /// Working settings that have not been published yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_settings: Option<Value>,
    /// Number of the last published settings revision.
    pub settings_revision: i64,
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
    /// Validated assets manifest. Normally loaded from the bundle by the API.
    #[serde(default)]
    pub assets_manifest: Option<Value>,
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

/// Publication state of a stored settings revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeSettingsRevisionStatus {
    /// A working copy that has not been published.
    Draft,
    /// A snapshot that was published to the storefront.
    Published,
}

impl ThemeSettingsRevisionStatus {
    /// Returns the storage representation.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }

    /// Parses a stored status, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            _ => None,
        }
    }
}

/// One stored theme settings revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeSettingsRevision {
    /// Revision identifier.
    pub id: String,
    /// Owning theme.
    pub theme_id: ThemeId,
    /// Monotonically increasing revision number per theme.
    pub revision: i64,
    /// Settings snapshot.
    pub settings: Value,
    /// Whether the snapshot is a draft or published revision.
    pub status: ThemeSettingsRevisionStatus,
    /// Actor that produced the revision, when known.
    pub created_by: Option<String>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
}

/// A runtime capability endpoint bound to a theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeCapabilityBinding {
    /// Binding identifier.
    pub id: String,
    /// Owning theme.
    pub theme_id: ThemeId,
    /// Capability identifier declared by the theme manifest.
    pub capability_id: String,
    /// Absolute URL serving the capability.
    pub endpoint: String,
    /// Capability-specific configuration; defaults to `{}`.
    pub config: Value,
    /// Whether the binding is currently served to the theme runtime.
    pub is_active: bool,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
    /// Last update timestamp in Unix seconds.
    pub updated_at: Timestamp,
}

/// Input for creating or replacing a capability binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeCapabilityBindingInput {
    /// Capability identifier declared by the theme manifest.
    pub capability_id: String,
    /// Absolute URL serving the capability.
    pub endpoint: String,
    /// Capability-specific configuration; must be a JSON object.
    #[serde(default = "empty_config")]
    pub config: Value,
    /// Whether the binding is active; defaults to `true`.
    #[serde(default = "default_active")]
    pub is_active: bool,
}

fn empty_config() -> Value {
    Value::Object(serde_json::Map::new())
}

const fn default_active() -> bool {
    true
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
