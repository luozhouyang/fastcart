//! Controlled plugin extension contracts and manifest registry.
//!
//! Plugins declare the capabilities they require. The host validates manifests
//! and resolves dependency order before enabling a plugin. Plugin code is never
//! executed while merely registering a manifest.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Host extension API version supported by this build.
pub const SUPPORTED_PLUGIN_API_VERSION: u32 = 1;

/// Capabilities a plugin may request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Optional entrypoint. Interpreted by the host, never by this registry.
    pub entrypoint: Option<String>,
    /// Plugin configuration schema.
    pub config_schema: Option<serde_json::Value>,
}

/// Plugin registry errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PluginError {
    /// The manifest is structurally invalid.
    #[error("invalid plugin manifest: {0}")]
    InvalidManifest(String),
    /// A plugin with the same id is already registered.
    #[error("plugin already registered: {0}")]
    Duplicate(String),
    /// The plugin is not registered.
    #[error("plugin not found: {0}")]
    NotFound(String),
    /// The plugin requires a host API this build does not provide.
    #[error("unsupported plugin api version: {0}")]
    UnsupportedApiVersion(u32),
    /// A dependency is missing.
    #[error("missing plugin dependency: {0}")]
    MissingDependency(String),
    /// The dependency graph contains a cycle.
    #[error("plugin dependency cycle: {0}")]
    DependencyCycle(String),
    /// The plugin lacks a required capability.
    #[error("plugin {plugin} lacks capability {capability:?}")]
    MissingCapability {
        /// Plugin identifier.
        plugin: String,
        /// Required capability.
        capability: Capability,
    },
}

impl PluginManifest {
    /// Validates identifiers, version, host API compatibility and capabilities.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.api_version != SUPPORTED_PLUGIN_API_VERSION {
            return Err(PluginError::UnsupportedApiVersion(self.api_version));
        }
        if !valid_identifier(&self.id) {
            return Err(PluginError::InvalidManifest(format!(
                "invalid plugin id: {}",
                self.id
            )));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(PluginError::InvalidManifest(
                "plugin name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if !valid_version(&self.version) {
            return Err(PluginError::InvalidManifest(format!(
                "invalid plugin version: {}",
                self.version
            )));
        }
        let mut seen = HashSet::new();
        for dependency in &self.dependencies {
            if !valid_identifier(dependency) {
                return Err(PluginError::InvalidManifest(format!(
                    "invalid dependency id: {dependency}"
                )));
            }
            if dependency == &self.id {
                return Err(PluginError::InvalidManifest(
                    "plugin cannot depend on itself".to_owned(),
                ));
            }
            if !seen.insert(dependency) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate dependency: {dependency}"
                )));
            }
        }
        let mut capabilities = HashSet::new();
        for capability in &self.capabilities {
            if !capabilities.insert(*capability) {
                return Err(PluginError::InvalidManifest(format!(
                    "duplicate capability: {capability:?}"
                )));
            }
        }
        Ok(())
    }

    /// Returns true when the manifest requests the capability.
    #[must_use]
    pub fn has_capability(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// In-memory registry of manifests with deterministic dependency ordering.
#[derive(Clone, Debug, Default)]
pub struct PluginRegistry {
    manifests: HashMap<String, PluginManifest>,
    enabled: HashSet<String>,
}

impl PluginRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers and validates a manifest.
    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), PluginError> {
        manifest.validate()?;
        if self.manifests.contains_key(&manifest.id) {
            return Err(PluginError::Duplicate(manifest.id));
        }
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Removes a disabled plugin.
    pub fn unregister(&mut self, id: &str) -> Result<(), PluginError> {
        if self.enabled.contains(id) {
            return Err(PluginError::InvalidManifest(format!(
                "plugin {id} must be disabled before removal"
            )));
        }
        self.manifests
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| PluginError::NotFound(id.to_owned()))
    }

    /// Returns a manifest.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&PluginManifest> {
        self.manifests.get(id)
    }

    /// Lists registered manifests sorted by id.
    #[must_use]
    pub fn list(&self) -> Vec<&PluginManifest> {
        let mut manifests: Vec<&PluginManifest> = self.manifests.values().collect();
        manifests.sort_by(|left, right| left.id.cmp(&right.id));
        manifests
    }

    /// Enables a plugin after verifying that all dependencies are registered
    /// and resolvable.
    pub fn enable(&mut self, id: &str) -> Result<(), PluginError> {
        if !self.manifests.contains_key(id) {
            return Err(PluginError::NotFound(id.to_owned()));
        }
        self.resolve_order()?;
        self.enabled.insert(id.to_owned());
        Ok(())
    }

    /// Disables a plugin. Plugins that depend on it must be disabled first.
    pub fn disable(&mut self, id: &str) -> Result<(), PluginError> {
        if !self.enabled.contains(id) {
            return Ok(());
        }
        let dependents: Vec<&str> = self
            .manifests
            .values()
            .filter(|manifest| {
                manifest
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == id)
                    && self.enabled.contains(&manifest.id)
            })
            .map(|manifest| manifest.id.as_str())
            .collect();
        if !dependents.is_empty() {
            return Err(PluginError::InvalidManifest(format!(
                "plugin {id} is required by: {}",
                dependents.join(", ")
            )));
        }
        self.enabled.remove(id);
        Ok(())
    }

    /// Returns true when a plugin is enabled.
    #[must_use]
    pub fn is_enabled(&self, id: &str) -> bool {
        self.enabled.contains(id)
    }

    /// Returns enabled plugins in dependency order.
    pub fn enabled_order(&self) -> Result<Vec<&PluginManifest>, PluginError> {
        Ok(self
            .resolve_order()?
            .into_iter()
            .filter(|manifest| self.enabled.contains(&manifest.id))
            .collect())
    }

    /// Resolves every registered manifest in dependency order.
    pub fn resolve_order(&self) -> Result<Vec<&PluginManifest>, PluginError> {
        let mut ordered = Vec::with_capacity(self.manifests.len());
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut ids: Vec<&String> = self.manifests.keys().collect();
        ids.sort();
        for id in ids {
            self.visit(id, &mut visiting, &mut visited, &mut ordered)?;
        }
        Ok(ordered)
    }

    fn visit<'a>(
        &'a self,
        id: &'a str,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a PluginManifest>,
    ) -> Result<(), PluginError> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.to_owned()) {
            return Err(PluginError::DependencyCycle(id.to_owned()));
        }
        let manifest = self
            .manifests
            .get(id)
            .ok_or_else(|| PluginError::MissingDependency(id.to_owned()))?;
        for dependency in &manifest.dependencies {
            self.visit(dependency, visiting, visited, ordered)?;
        }
        visiting.remove(id);
        visited.insert(id.to_owned());
        ordered.push(manifest);
        Ok(())
    }

    /// Verifies that an enabled plugin holds the required capability.
    pub fn require_capability(&self, id: &str, capability: Capability) -> Result<(), PluginError> {
        let manifest = self
            .manifests
            .get(id)
            .ok_or_else(|| PluginError::NotFound(id.to_owned()))?;
        if manifest.has_capability(capability) {
            Ok(())
        } else {
            Err(PluginError::MissingCapability {
                plugin: id.to_owned(),
                capability,
            })
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && !value.starts_with('-')
        && !value.starts_with('.')
        && !value.starts_with('_')
        && !value.ends_with('-')
        && !value.ends_with('.')
        && !value.ends_with('_')
}

fn valid_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Convenience constructor for manifests in tests and examples.
#[must_use]
pub fn manifest(id: &str, name: &str, version: &str) -> PluginManifest {
    PluginManifest {
        id: id.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        api_version: SUPPORTED_PLUGIN_API_VERSION,
        capabilities: Vec::new(),
        dependencies: Vec::new(),
        entrypoint: None,
        config_schema: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, PluginError, PluginManifest, PluginRegistry, manifest};

    fn with_dependencies(id: &str, dependencies: &[&str]) -> PluginManifest {
        let mut manifest = manifest(id, id, "1.0.0");
        manifest.dependencies = dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
        manifest
    }

    #[test]
    fn validates_manifests() {
        let mut registry = PluginRegistry::new();
        assert!(
            registry
                .register(manifest("my-plugin", "My Plugin", "1.0.0"))
                .is_ok()
        );

        let mut invalid = manifest("Bad Id", "Bad", "1.0.0");
        invalid.id = "Bad Id".to_owned();
        assert!(matches!(
            registry.register(invalid),
            Err(PluginError::InvalidManifest(_))
        ));

        let mut unsupported = manifest("future", "Future", "1.0.0");
        unsupported.api_version = 99;
        assert_eq!(
            registry.register(unsupported),
            Err(PluginError::UnsupportedApiVersion(99))
        );
    }

    #[test]
    fn duplicate_plugins_are_rejected() {
        let mut registry = PluginRegistry::new();
        registry.register(manifest("a", "A", "1.0.0")).unwrap();
        assert_eq!(
            registry.register(manifest("a", "A", "1.0.0")),
            Err(PluginError::Duplicate("a".to_owned()))
        );
    }

    #[test]
    fn resolves_dependencies_in_order() {
        let mut registry = PluginRegistry::new();
        registry
            .register(with_dependencies("app", &["core"]))
            .unwrap();
        registry
            .register(with_dependencies("core", &["base"]))
            .unwrap();
        registry.register(with_dependencies("base", &[])).unwrap();

        let order: Vec<&str> = registry
            .resolve_order()
            .unwrap()
            .iter()
            .map(|manifest| manifest.id.as_str())
            .collect();
        assert_eq!(order, vec!["base", "core", "app"]);
    }

    #[test]
    fn detects_cycles_and_missing_dependencies() {
        let mut registry = PluginRegistry::new();
        registry.register(with_dependencies("a", &["b"])).unwrap();
        registry.register(with_dependencies("b", &["a"])).unwrap();
        assert!(matches!(
            registry.resolve_order(),
            Err(PluginError::DependencyCycle(_))
        ));

        let mut registry = PluginRegistry::new();
        registry
            .register(with_dependencies("a", &["missing"]))
            .unwrap();
        assert_eq!(
            registry.resolve_order(),
            Err(PluginError::MissingDependency("missing".to_owned()))
        );
    }

    #[test]
    fn enable_disable_respects_dependents() {
        let mut registry = PluginRegistry::new();
        registry.register(with_dependencies("core", &[])).unwrap();
        registry
            .register(with_dependencies("app", &["core"]))
            .unwrap();
        registry.enable("core").unwrap();
        registry.enable("app").unwrap();

        assert!(registry.disable("core").is_err());
        registry.disable("app").unwrap();
        registry.disable("core").unwrap();
        assert!(!registry.is_enabled("core"));
    }

    #[test]
    fn capabilities_are_enforced() {
        let mut registry = PluginRegistry::new();
        let mut plugin = manifest("events", "Events", "1.0.0");
        plugin.capabilities = vec![Capability::EventHandler];
        registry.register(plugin).unwrap();

        assert!(
            registry
                .require_capability("events", Capability::EventHandler)
                .is_ok()
        );
        assert!(matches!(
            registry.require_capability("events", Capability::ApiExtension),
            Err(PluginError::MissingCapability { .. })
        ));
    }
}
