//! Domain entities and inputs for the theme registry.

use fastcart_kernel::{ThemeActivationId, ThemeId, Timestamp};
use fastcart_storefront::ThemeManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A registered storefront theme.
///
/// The record is metadata plus settings and activation state. Rendering and
/// assets belong to the deployed theme artifact (ADR 0008).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Internal theme identifier.
    pub id: ThemeId,
    /// Parsed and validated theme manifest metadata.
    pub manifest: ThemeManifest,
    /// JSON Schema used to validate theme settings.
    pub settings_schema: Option<Value>,
    /// Current theme settings.
    pub settings: Value,
    /// Working settings that have not been published yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_settings: Option<Value>,
    /// Number of the last published settings revision.
    pub settings_revision: i64,
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
    /// Theme manifest metadata to validate.
    pub manifest: ThemeManifest,
    /// Optional JSON Schema for theme settings. Must be a JSON object.
    #[serde(default)]
    pub settings_schema: Option<Value>,
    /// Initial settings; defaults to an empty object.
    #[serde(default)]
    pub settings: Option<Value>,
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

/// An activation history entry enriched with the activated theme's metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeActivationRecord {
    /// Activation identifier.
    pub id: ThemeActivationId,
    /// Theme that was activated.
    pub theme_id: ThemeId,
    /// Manifest identifier of the activated theme, when it still exists.
    pub manifest_id: Option<String>,
    /// Display name of the activated theme, when it still exists.
    pub theme_name: Option<String>,
    /// Version of the activated theme, when it still exists.
    pub theme_version: Option<String>,
    /// Theme that was active before this activation, when any.
    pub previous_theme_id: Option<ThemeId>,
    /// Creation timestamp in Unix seconds.
    pub created_at: Timestamp,
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
