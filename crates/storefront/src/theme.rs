//! Theme manifest contract.

/// Describes a trusted storefront theme bundle.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ThemeManifest {
    /// Stable theme identifier.
    pub id: String,
    /// Human-readable theme name.
    pub name: String,
    /// Theme version.
    pub version: String,
    /// Rendering runtime expected by the theme.
    pub runtime: String,
    /// Entry file inside the theme bundle.
    pub entry: String,
    /// Optional JSON Schema for theme settings.
    pub settings_schema: Option<String>,
    /// Optional asset manifest file.
    pub assets_manifest: Option<String>,
}
