//! The `theme` module: storefront theme registry and activation.

pub mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{RegisterThemeInput, Theme, ThemeActivation, UpdateThemeSettingsInput};
pub use service::ThemeService;
