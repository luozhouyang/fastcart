//! Storefront theme and page rendering contracts.
//!
//! Themes are deployment artifacts (ADR 0008): only their metadata is stored in
//! the registry. Pages are data, and rendering resolves section types through a
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
pub use theme::{SUPPORTED_EDITOR_SCHEMA_VERSION, ThemeManifest};

/// Validation errors raised by storefront contracts.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorefrontError {
    /// The page or theme manifest is structurally invalid.
    #[error("invalid storefront definition: {0}")]
    Invalid(String),
}
