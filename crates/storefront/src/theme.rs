//! Typed theme manifest validation.
//!
//! Themes are deployment artifacts (ADR 0008); the registry only stores this
//! metadata, a settings schema and settings. Rendering and assets belong to the
//! deployed SSR artifact.

use std::collections::HashSet;

use crate::StorefrontError;

/// Editor manifest schema version implemented by this build.
pub const SUPPORTED_EDITOR_SCHEMA_VERSION: u32 = 1;

/// Data API version implemented by this build.
pub const SUPPORTED_DATA_API_VERSION: u32 = 1;

/// Metadata describing a registered storefront theme.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeManifest {
    /// Stable theme identifier.
    pub id: String,
    /// Human-readable theme name.
    pub name: String,
    /// Theme version.
    pub version: String,
    /// Platform and plugin capabilities the theme needs at activation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<ThemeRequirements>,
}

/// Platform contracts a theme declares before activation.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeRequirements {
    /// Data API version the theme was written against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_api: Option<u32>,
    /// Plugin capabilities the theme requires.
    #[serde(default)]
    pub capabilities: Vec<ThemeCapabilityRequirement>,
}

impl ThemeRequirements {
    fn validate(&self) -> Result<(), StorefrontError> {
        if let Some(version) = self.data_api
            && !(1..=SUPPORTED_DATA_API_VERSION).contains(&version)
        {
            return Err(StorefrontError::Invalid(format!(
                "unsupported data API version: {version}"
            )));
        }

        let mut seen = HashSet::new();
        for capability in &self.capabilities {
            if !valid_identifier(&capability.id) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid capability id: {}",
                    capability.id
                )));
            }
            if capability.version == Some(0) {
                return Err(StorefrontError::Invalid(format!(
                    "invalid capability version for {}: 0",
                    capability.id
                )));
            }
            if !seen.insert((capability.id.as_str(), capability.version)) {
                return Err(StorefrontError::Invalid(format!(
                    "duplicate capability requirement: {}",
                    capability.id
                )));
            }
        }
        Ok(())
    }
}

/// One plugin capability a theme requires.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeCapabilityRequirement {
    /// Capability identifier.
    pub id: String,
    /// Exact provider version required, when pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Whether activation may continue when the capability is missing.
    #[serde(default)]
    pub optional: bool,
}

/// A capability offered by an available provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableCapability {
    /// Capability identifier.
    pub id: String,
    /// Provided capability version.
    pub version: u32,
}

impl ThemeManifest {
    /// Validates the theme identifier, name, version and requirements.
    pub fn validate(&self) -> Result<(), StorefrontError> {
        if !valid_identifier(&self.id) {
            return Err(StorefrontError::Invalid(format!(
                "invalid theme id: {}",
                self.id
            )));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(StorefrontError::Invalid(
                "theme name must be between 1 and 120 characters".to_owned(),
            ));
        }
        if !valid_version(&self.version) {
            return Err(StorefrontError::Invalid(format!(
                "invalid theme version: {}",
                self.version
            )));
        }
        if let Some(requires) = self.requires.as_ref() {
            requires.validate()?;
        }
        Ok(())
    }

    /// Required (non-optional) capabilities that no available provider
    /// satisfies.
    ///
    /// A requirement is satisfied by an available capability with the same
    /// identifier and, when the requirement pins a version, the same version.
    #[must_use]
    pub fn missing_capabilities(
        &self,
        available: &[AvailableCapability],
    ) -> Vec<ThemeCapabilityRequirement> {
        let Some(requires) = self.requires.as_ref() else {
            return Vec::new();
        };
        requires
            .capabilities
            .iter()
            .filter(|requirement| !requirement.optional)
            .filter(|requirement| {
                !available.iter().any(|capability| {
                    capability.id == requirement.id
                        && requirement
                            .version
                            .is_none_or(|version| capability.version == version)
                })
            })
            .cloned()
            .collect()
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn valid_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{
        AvailableCapability, SUPPORTED_DATA_API_VERSION, ThemeCapabilityRequirement, ThemeManifest,
        ThemeRequirements,
    };

    fn theme() -> ThemeManifest {
        ThemeManifest {
            id: "default".to_owned(),
            name: "Default".to_owned(),
            version: "1.0.0".to_owned(),
            requires: None,
        }
    }

    fn capability(id: &str, version: Option<u32>, optional: bool) -> ThemeCapabilityRequirement {
        ThemeCapabilityRequirement {
            id: id.to_owned(),
            version,
            optional,
        }
    }

    fn available(id: &str, version: u32) -> AvailableCapability {
        AvailableCapability {
            id: id.to_owned(),
            version,
        }
    }

    fn with_requirements(requirements: ThemeRequirements) -> ThemeManifest {
        ThemeManifest {
            requires: Some(requirements),
            ..theme()
        }
    }

    #[test]
    fn valid_themes_pass() {
        theme().validate().unwrap();
    }

    #[test]
    fn valid_requirements_pass() {
        let manifest = with_requirements(ThemeRequirements {
            data_api: Some(SUPPORTED_DATA_API_VERSION),
            capabilities: vec![
                capability("catalog.search", Some(2), false),
                capability("reviews", None, true),
                // The same identifier at another version is a distinct pair.
                capability("catalog.search", Some(1), true),
            ],
        });
        manifest.validate().unwrap();
    }

    #[test]
    fn invalid_identifiers_names_and_versions_are_rejected() {
        let mut manifest = theme();
        manifest.id = "Bad Theme".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.name = "   ".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.name = "x".repeat(121);
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.version = "one".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.version = "1.0.0.0".to_owned();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn unsupported_data_api_versions_are_rejected() {
        for version in [0, SUPPORTED_DATA_API_VERSION + 1] {
            let manifest = with_requirements(ThemeRequirements {
                data_api: Some(version),
                capabilities: Vec::new(),
            });
            assert!(manifest.validate().is_err(), "accepted {version}");
        }
    }

    #[test]
    fn invalid_capability_identifiers_are_rejected() {
        for id in ["Bad", "", "-leading", "trailing-", &"x".repeat(121)] {
            let manifest = with_requirements(ThemeRequirements {
                data_api: None,
                capabilities: vec![capability(id, Some(1), false)],
            });
            assert!(manifest.validate().is_err(), "accepted {id:?}");
        }
    }

    #[test]
    fn zero_capability_versions_are_rejected() {
        let manifest = with_requirements(ThemeRequirements {
            data_api: None,
            capabilities: vec![capability("reviews", Some(0), false)],
        });
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn duplicate_capability_pairs_are_rejected() {
        let manifest = with_requirements(ThemeRequirements {
            data_api: None,
            capabilities: vec![
                capability("payments", Some(2), false),
                capability("payments", Some(2), true),
            ],
        });
        assert!(manifest.validate().is_err());

        let manifest = with_requirements(ThemeRequirements {
            data_api: None,
            capabilities: vec![
                capability("payments", None, false),
                capability("payments", None, false),
            ],
        });
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn missing_capabilities_reports_unsatisfied_required_requirements() {
        let manifest = with_requirements(ThemeRequirements {
            data_api: Some(SUPPORTED_DATA_API_VERSION),
            capabilities: vec![
                capability("catalog.search", Some(2), false),
                capability("payments", None, false),
                capability("reviews", Some(1), false),
                capability("wishlist", None, true),
            ],
        });

        let missing = manifest.missing_capabilities(&[
            // Exact identifier and version match.
            available("catalog.search", 2),
            // A versionless requirement accepts any version.
            available("payments", 7),
            // Optional requirements never surface as missing.
            available("wishlist", 1),
        ]);
        assert_eq!(missing, vec![capability("reviews", Some(1), false)]);

        // A pinned version is not satisfied by another provider version.
        let missing = manifest.missing_capabilities(&[available("catalog.search", 1)]);
        assert_eq!(
            missing,
            vec![
                capability("catalog.search", Some(2), false),
                capability("payments", None, false),
                capability("reviews", Some(1), false),
            ]
        );
    }

    #[test]
    fn missing_capabilities_is_empty_without_requirements() {
        assert!(theme().missing_capabilities(&[]).is_empty());
        assert!(
            with_requirements(ThemeRequirements::default())
                .missing_capabilities(&[])
                .is_empty()
        );
    }
}
