//! The `theme` module: storefront theme registry and activation.

mod bundle;
pub mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use bundle::{
    bundle_directory_exists, load_assets_manifest, load_editor_manifest, load_settings_schema,
    load_theme_bundle_file,
};
pub use domain::{
    RegisterThemeInput, Theme, ThemeActivation, ThemeCapabilityBinding,
    ThemeCapabilityBindingInput, ThemeSettingsRevision, ThemeSettingsRevisionStatus,
    UpdateThemeSettingsInput,
};
pub use service::ThemeService;
