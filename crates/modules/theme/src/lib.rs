//! The `theme` module: storefront theme settings registry and activation.
//!
//! Themes are deployment artifacts (ADR 0008). This module owns the registry
//! metadata, the settings schema, settings revisions and activation history;
//! rendering and assets belong to a deployed SSR artifact.

pub mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    RegisterThemeInput, Theme, ThemeActivation, ThemeActivationRecord, ThemeSettingsRevision,
    ThemeSettingsRevisionStatus, UpdateThemeSettingsInput,
};
pub use service::ThemeService;
