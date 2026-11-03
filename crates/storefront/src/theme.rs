//! Typed theme manifest validation.

use crate::{StorefrontError, validate_bundle_path};

/// Rendering runtimes supported by the storefront.
pub const SUPPORTED_RUNTIMES: [&str; 2] = ["v8", "node"];

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
    pub settings_schema: Option<String>,
    /// Optional asset manifest file.
    pub assets_manifest: Option<String>,
}

impl ThemeManifest {
    /// Validates identifiers, runtime and bundle paths.
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
}
