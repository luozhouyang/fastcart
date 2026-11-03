//! Controlled plugin extension contracts.

/// Capabilities a plugin may request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Subscribe to domain or integration events.
    EventHandler,
    /// Register navigation entries or pages in the admin application.
    AdminExtension,
    /// Register additional API routes.
    ApiExtension,
    /// Register a provider implementation.
    Provider,
    /// Contribute trusted storefront components.
    StorefrontComponent,
}

/// Declares a plugin and the capabilities it requires.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PluginManifest {
    /// Stable plugin identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Plugin version.
    pub version: String,
    /// Host extension API version.
    pub api_version: u32,
    /// Requested capabilities.
    pub capabilities: Vec<Capability>,
    /// Other plugins this plugin depends on.
    pub dependencies: Vec<String>,
}
