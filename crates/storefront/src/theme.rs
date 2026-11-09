//! Typed theme manifest validation.
//!
//! Themes are deployment artifacts (ADR 0008); the registry only stores this
//! metadata, a settings schema and settings. Rendering and assets belong to the
//! deployed SSR artifact.

use crate::StorefrontError;

/// Editor manifest schema version implemented by this build.
pub const SUPPORTED_EDITOR_SCHEMA_VERSION: u32 = 1;

/// Metadata describing a registered storefront theme.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThemeManifest {
    /// Stable theme identifier.
    pub id: String,
    /// Human-readable theme name.
    pub name: String,
    /// Theme version.
    pub version: String,
}

impl ThemeManifest {
    /// Validates the theme identifier, name and version.
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
        }
    }

    #[test]
    fn valid_themes_pass() {
        theme().validate().unwrap();
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
}
