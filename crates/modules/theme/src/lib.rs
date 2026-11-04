//! The `theme` module: storefront theme registry and activation.

mod bundle;
pub mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use bundle::load_editor_manifest;
pub use domain::{RegisterThemeInput, Theme, ThemeActivation, UpdateThemeSettingsInput};
pub use service::ThemeService;
