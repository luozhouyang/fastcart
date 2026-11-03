//! Storefront theme and page rendering contracts.
//!
//! Themes and pages are data. Rendering resolves section types through a
//! trusted host registry; stored page JSON never contains executable code.

pub mod page;
pub mod theme;

pub use page::{PageSchema, PageSection};
pub use theme::ThemeManifest;

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
pub fn validate_bundle_path(path: &str) -> Result<(), StorefrontError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
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
    }
}
