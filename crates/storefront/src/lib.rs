//! Storefront theme and page rendering contracts.
//!
//! Themes and pages are data. Rendering resolves section types through a
//! trusted host registry; stored page JSON never contains executable code.

pub mod data;
pub mod editor;
pub mod page;
pub mod theme;

pub use data::{BindingRequest, DataBag, DataBinding};
pub use editor::{
    EditorCategory, EditorComponent, EditorField, EditorFieldOption, EditorRoot, EditorSlot,
    ThemeEditorManifest,
};
pub use page::{PageBlock, PageBody, PageRoot, PageThemeRef};
pub use theme::{
    CapabilityRequirement, SUPPORTED_DATA_API_VERSION, SUPPORTED_EDITOR_SCHEMA_VERSION,
    ThemeEditorRef, ThemeManifest, ThemeRequirements,
};

/// Validation errors raised by storefront contracts.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorefrontError {
    /// The page or theme manifest is structurally invalid.
    #[error("invalid storefront definition: {0}")]
    Invalid(String),
    /// A path escapes its bundle root.
    #[error("unsafe bundle path: {0}")]
    UnsafePath(String),
}

/// Validates a bundle-relative path.
///
/// Paths must be relative, must not contain `..`, and may not start with `/`.
/// Drive prefixes (`C:...`), Windows separators and control characters are
/// rejected so a bundle path cannot be reinterpreted as an absolute path when
/// the runtime joins it to the bundle root.
pub fn validate_bundle_path(path: &str) -> Result<(), StorefrontError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || std::path::Path::new(path).is_absolute()
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(StorefrontError::UnsafePath(path.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_bundle_path;

    #[test]
    fn bundle_paths_cannot_escape_the_root() {
        assert!(validate_bundle_path("assets/main.js").is_ok());
        assert!(validate_bundle_path("../secret").is_err());
        assert!(validate_bundle_path("/etc/passwd").is_err());
        assert!(validate_bundle_path("assets//main.js").is_err());
        assert!(validate_bundle_path("assets\\main.js").is_err());
        assert!(validate_bundle_path("C:/Windows/system32").is_err());
        assert!(validate_bundle_path("C:secret.js").is_err());
        assert!(validate_bundle_path("assets/\u{7}main.js").is_err());
    }
}
