//! Typed theme manifest validation.

use crate::{StorefrontError, validate_bundle_path};

/// Rendering runtimes supported by the storefront.
pub const SUPPORTED_RUNTIMES: [&str; 2] = ["v8", "node"];

/// Storefront data API contract version implemented by this build.
pub const SUPPORTED_DATA_API_VERSION: u32 = 1;

/// Editor manifest schema version implemented by this build.
pub const SUPPORTED_EDITOR_SCHEMA_VERSION: u32 = 1;

/// Describes a trusted storefront theme bundle.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings_schema: Option<String>,
    /// Optional asset manifest file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets_manifest: Option<String>,
    /// Optional editor bundle and manifest references.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<ThemeEditorRef>,
    /// Optional platform contracts and capabilities the theme depends on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<ThemeRequirements>,
}

/// Bundle-relative references to the theme editor surface.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeEditorRef {
    /// ESM entry that exports render and field components.
    pub entry: String,
    /// JSON file describing categories, components and fields.
    pub manifest: String,
    /// Editor manifest schema version.
    #[serde(default = "default_editor_schema_version")]
    pub schema_version: u32,
}

/// Platform contracts and capabilities a theme depends on.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeRequirements {
    /// Required storefront data API version.
    #[serde(default = "default_data_api_version")]
    pub data_api: u32,
    /// Optional or required plugin capabilities.
    #[serde(default)]
    pub capabilities: Vec<CapabilityRequirement>,
}

/// One capability required by a theme.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityRequirement {
    /// Capability identifier.
    pub id: String,
    /// Required capability version.
    pub version: u32,
    /// Whether activation may proceed when the capability is missing.
    #[serde(default)]
    pub optional: bool,
}

const fn default_data_api_version() -> u32 {
    SUPPORTED_DATA_API_VERSION
}

const fn default_editor_schema_version() -> u32 {
    SUPPORTED_EDITOR_SCHEMA_VERSION
}

impl ThemeManifest {
    /// Validates identifiers, runtime, bundle paths and platform contracts.
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
        if !SUPPORTED_RUNTIMES.contains(&self.runtime.as_str()) {
            return Err(StorefrontError::Invalid(format!(
                "unsupported theme runtime: {}",
                self.runtime
            )));
        }
        validate_bundle_path(&self.entry)?;
        for path in [&self.settings_schema, &self.assets_manifest]
            .into_iter()
            .flatten()
        {
            validate_bundle_path(path)?;
        }
        if let Some(editor) = self.editor.as_ref() {
            validate_bundle_path(&editor.entry)?;
            validate_bundle_path(&editor.manifest)?;
            if editor.schema_version != SUPPORTED_EDITOR_SCHEMA_VERSION {
                return Err(StorefrontError::Invalid(format!(
                    "unsupported editor manifest schema version: {}",
                    editor.schema_version
                )));
            }
        }
        if let Some(requires) = self.requires.as_ref() {
            if requires.data_api == 0 || requires.data_api > SUPPORTED_DATA_API_VERSION {
                return Err(StorefrontError::Invalid(format!(
                    "unsupported storefront data API version: {}",
                    requires.data_api
                )));
            }
            let mut seen = std::collections::HashSet::new();
            for capability in &requires.capabilities {
                if !valid_identifier(&capability.id) {
                    return Err(StorefrontError::Invalid(format!(
                        "invalid capability id: {}",
                        capability.id
                    )));
                }
                if capability.version == 0 {
                    return Err(StorefrontError::Invalid(
                        "capability version must be greater than zero".to_owned(),
                    ));
                }
                if !seen.insert(capability.id.as_str()) {
                    return Err(StorefrontError::Invalid(format!(
                        "duplicate capability requirement: {}",
                        capability.id
                    )));
                }
            }
        }
        Ok(())
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
    use super::ThemeManifest;

    fn theme() -> ThemeManifest {
        ThemeManifest {
            id: "default".to_owned(),
            name: "Default".to_owned(),
            version: "1.0.0".to_owned(),
            runtime: "v8".to_owned(),
            entry: "dist/server.js".to_owned(),
            settings_schema: Some("settings.schema.json".to_owned()),
            assets_manifest: Some("assets.json".to_owned()),
            editor: None,
            requires: None,
        }
    }

    #[test]
    fn valid_themes_pass() {
        theme().validate().unwrap();
    }

    #[test]
    fn invalid_identifiers_and_runtimes_are_rejected() {
        let mut manifest = theme();
        manifest.id = "Bad Theme".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.runtime = "browser".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.version = "one".to_owned();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn entries_cannot_escape_the_bundle() {
        let mut manifest = theme();
        manifest.entry = "../outside.js".to_owned();
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.assets_manifest = Some("/absolute.json".to_owned());
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn editor_references_and_contracts_are_validated() {
        use super::{CapabilityRequirement, ThemeEditorRef, ThemeRequirements};

        let mut manifest = theme();
        manifest.editor = Some(ThemeEditorRef {
            entry: "editor.js".to_owned(),
            manifest: "editor.manifest.json".to_owned(),
            schema_version: 1,
        });
        manifest.requires = Some(ThemeRequirements {
            data_api: 1,
            capabilities: vec![CapabilityRequirement {
                id: "chat".to_owned(),
                version: 1,
                optional: true,
            }],
        });
        manifest.validate().unwrap();

        let mut manifest = theme();
        manifest.editor = Some(ThemeEditorRef {
            entry: "../escape.js".to_owned(),
            manifest: "editor.manifest.json".to_owned(),
            schema_version: 1,
        });
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.editor = Some(ThemeEditorRef {
            entry: "editor.js".to_owned(),
            manifest: "editor.manifest.json".to_owned(),
            schema_version: 2,
        });
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.requires = Some(ThemeRequirements {
            data_api: 2,
            capabilities: Vec::new(),
        });
        assert!(manifest.validate().is_err());

        let mut manifest = theme();
        manifest.requires = Some(ThemeRequirements {
            data_api: 1,
            capabilities: vec![CapabilityRequirement {
                id: "Bad Capability".to_owned(),
                version: 1,
                optional: false,
            }],
        });
        assert!(manifest.validate().is_err());
    }
}
