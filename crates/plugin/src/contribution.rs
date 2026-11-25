//! Declarative contributions a plugin provides to the host.
//!
//! A manifest may declare contributions such as storefront capabilities and
//! storefront components. The host only indexes and validates these
//! declarations; it never loads or executes plugin code because of them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::PluginManifest;

/// One declarative contribution provided by a plugin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Contribution {
    /// A storefront capability other contributions can require.
    StorefrontCapability(CapabilityContribution),
    /// A storefront component rendered by the host or a theme.
    StorefrontComponent(ComponentContribution),
}

impl Contribution {
    /// Stable kind string used in manifests and the contribution index.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::StorefrontCapability(_) => "storefront_capability",
            Self::StorefrontComponent(_) => "storefront_component",
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::StorefrontCapability(contribution) => &contribution.id,
            Self::StorefrontComponent(contribution) => &contribution.id,
        }
    }

    fn version(&self) -> Option<u32> {
        match self {
            Self::StorefrontCapability(contribution) => Some(contribution.version),
            Self::StorefrontComponent(_) => None,
        }
    }
}

/// A storefront capability a plugin provides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityContribution {
    /// Capability identifier.
    pub id: String,
    /// Capability version; starts at `1`.
    pub version: u32,
    /// Human-readable title.
    pub title: String,
    /// Optional JSON Schema for the capability configuration.
    #[serde(default)]
    pub config_schema: Option<Value>,
}

/// A storefront component a plugin provides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentContribution {
    /// Component identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Optional editor category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Optional editor metadata for host tooling.
    #[serde(default)]
    pub editor_manifest: Option<Value>,
    /// Optional component module shipped in the archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Capabilities the component requires.
    #[serde(default)]
    pub requires: Vec<CapabilityRequirement>,
}

/// A capability required by a storefront component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    /// Required capability identifier.
    pub id: String,
    /// Required capability version; any version when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Whether the component works without the capability.
    #[serde(default)]
    pub optional: bool,
}

/// A contribution together with the plugin that declares it.
#[derive(Clone, Debug, PartialEq)]
pub struct ContributionProvider {
    /// Identifier of the providing plugin.
    pub plugin_id: String,
    /// Display name of the providing plugin.
    pub plugin_name: String,
    /// Version of the providing plugin.
    pub plugin_version: String,
    /// The declared contribution.
    pub contribution: Contribution,
}

/// Index of contributions declared by installed plugins.
#[derive(Clone, Debug, Default)]
pub struct ContributionRegistry {
    capabilities: Vec<ContributionProvider>,
    components: Vec<ContributionProvider>,
}

impl ContributionRegistry {
    /// Builds a registry from plugin manifests keyed by plugin id.
    ///
    /// The manifests are consumed, so contributions move into the index
    /// instead of being cloned.
    #[must_use]
    pub fn from_plugins(plugins: impl IntoIterator<Item = (String, PluginManifest)>) -> Self {
        let mut capabilities = Vec::new();
        let mut components = Vec::new();
        for (plugin_id, manifest) in plugins {
            let PluginManifest {
                name,
                version,
                provides,
                ..
            } = manifest;
            for contribution in provides {
                push_provider(
                    &mut capabilities,
                    &mut components,
                    ContributionProvider {
                        plugin_id: plugin_id.clone(),
                        plugin_name: name.clone(),
                        plugin_version: version.clone(),
                        contribution,
                    },
                );
            }
        }
        finish(capabilities, components)
    }

    /// Builds a registry from borrowed manifests.
    ///
    /// Only the contributions and their provider metadata are cloned; the
    /// manifest stays owned by the caller. Callers that already hold parsed
    /// manifests, such as the cached plugin catalog, use this instead of
    /// cloning every manifest just to build the index.
    #[must_use]
    pub fn from_borrowed_plugins<'a>(
        plugins: impl IntoIterator<Item = (&'a str, &'a PluginManifest)>,
    ) -> Self {
        let mut capabilities = Vec::new();
        let mut components = Vec::new();
        for (plugin_id, manifest) in plugins {
            for contribution in &manifest.provides {
                push_provider(
                    &mut capabilities,
                    &mut components,
                    ContributionProvider {
                        plugin_id: plugin_id.to_owned(),
                        plugin_name: manifest.name.clone(),
                        plugin_version: manifest.version.clone(),
                        contribution: contribution.clone(),
                    },
                );
            }
        }
        finish(capabilities, components)
    }

    /// All capability contributions, ordered by plugin then id.
    #[must_use]
    pub fn capabilities(&self) -> &[ContributionProvider] {
        &self.capabilities
    }

    /// All component contributions, ordered by plugin then id.
    #[must_use]
    pub fn components(&self) -> &[ContributionProvider] {
        &self.components
    }

    /// Providers matching an exact id and, when supplied, version.
    #[must_use]
    pub fn find_capability(&self, id: &str, version: Option<u32>) -> Vec<&ContributionProvider> {
        self.capabilities
            .iter()
            .filter(|provider| match &provider.contribution {
                Contribution::StorefrontCapability(contribution) => {
                    contribution.id == id
                        && version.is_none_or(|expected| contribution.version == expected)
                }
                Contribution::StorefrontComponent(_) => false,
            })
            .collect()
    }

    /// Groups with more than one provider for the same `(kind, id, version)`.
    #[must_use]
    pub fn duplicates(&self) -> Vec<Vec<&ContributionProvider>> {
        let mut groups: BTreeMap<(&str, &str, Option<u32>), Vec<&ContributionProvider>> =
            BTreeMap::new();
        for provider in self.capabilities.iter().chain(&self.components) {
            let contribution = &provider.contribution;
            groups
                .entry((
                    contribution.kind(),
                    contribution.id(),
                    contribution.version(),
                ))
                .or_default()
                .push(provider);
        }
        groups
            .into_values()
            .filter(|providers| providers.len() > 1)
            .collect()
    }
}

fn finish(
    mut capabilities: Vec<ContributionProvider>,
    mut components: Vec<ContributionProvider>,
) -> ContributionRegistry {
    sort_providers(&mut capabilities);
    sort_providers(&mut components);
    ContributionRegistry {
        capabilities,
        components,
    }
}

fn push_provider(
    capabilities: &mut Vec<ContributionProvider>,
    components: &mut Vec<ContributionProvider>,
    provider: ContributionProvider,
) {
    if matches!(provider.contribution, Contribution::StorefrontCapability(_)) {
        capabilities.push(provider);
    } else {
        components.push(provider);
    }
}

fn sort_providers(providers: &mut [ContributionProvider]) {
    providers.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then_with(|| left.contribution.id().cmp(right.contribution.id()))
            .then_with(|| {
                left.contribution
                    .version()
                    .cmp(&right.contribution.version())
            })
    });
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn manifest(id: &str, provides: Value) -> PluginManifest {
        let value = json!({
            "schema_version": "1",
            "id": id,
            "name": format!("Plugin {id}"),
            "version": "1.0.0",
            "api_version": 2,
            "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
            "entry": { "type": "worker", "path": "worker/index.js" },
            "capabilities": { "storefront": ["storefront.component"] },
            "provides": provides
        });
        PluginManifest::parse(value.to_string().as_bytes()).expect("manifest parses")
    }

    fn registry() -> ContributionRegistry {
        let first = manifest(
            "plugin-a",
            json!([
                {
                    "kind": "storefront_capability",
                    "id": "search",
                    "version": 1,
                    "title": "Search"
                },
                {
                    "kind": "storefront_component",
                    "id": "hero",
                    "title": "Hero"
                }
            ]),
        );
        let second = manifest(
            "plugin-b",
            json!([
                {
                    "kind": "storefront_capability",
                    "id": "search",
                    "version": 2,
                    "title": "Search"
                },
                {
                    "kind": "storefront_capability",
                    "id": "search",
                    "version": 1,
                    "title": "Search"
                }
            ]),
        );
        ContributionRegistry::from_plugins([
            ("plugin-b".to_owned(), second),
            ("plugin-a".to_owned(), first),
        ])
    }

    #[test]
    fn indexes_contributions_by_kind_and_plugin() {
        let registry = registry();
        let capabilities = registry.capabilities();
        assert_eq!(capabilities.len(), 3);
        assert_eq!(capabilities[0].plugin_id, "plugin-a");
        assert_eq!(capabilities[0].contribution.kind(), "storefront_capability");
        assert_eq!(capabilities[1].plugin_id, "plugin-b");
        assert_eq!(capabilities[1].plugin_name, "Plugin plugin-b");

        let components = registry.components();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].plugin_id, "plugin-a");
        assert_eq!(components[0].contribution.kind(), "storefront_component");
    }

    #[test]
    fn finds_exact_capability_versions() {
        let registry = registry();
        let any = registry.find_capability("search", None);
        assert_eq!(any.len(), 3);

        let first = registry.find_capability("search", Some(1));
        assert_eq!(first.len(), 2);
        assert!(
            first.iter().all(
                |provider| provider.plugin_id == "plugin-a" || provider.plugin_id == "plugin-b"
            )
        );

        let second = registry.find_capability("search", Some(2));
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].plugin_id, "plugin-b");

        assert!(registry.find_capability("search", Some(9)).is_empty());
        assert!(registry.find_capability("unknown", None).is_empty());
    }

    #[test]
    fn detects_duplicates_across_plugins() {
        let registry = registry();
        let duplicates = registry.duplicates();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].len(), 2);
        let ids: Vec<&str> = duplicates[0]
            .iter()
            .map(|provider| provider.plugin_id.as_str())
            .collect();
        assert_eq!(ids, vec!["plugin-a", "plugin-b"]);
    }
}
