//! The `content` commerce module.
//!
//! Domain rules, application services and SQLite repositories for pages, page
//! versions, navigation, redirects and translations live here. The crate is
//! self-contained: cross-module orchestration belongs in `fastcart-workflows`.

mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_BODY_BYTES, Navigation,
    NavigationItem, NavigationItemInput, NavigationTree, NavigationTreeItem, Page,
    PageRevisionOrigin, PageRoute, PageStatus, PageVersion, PageVersionKind, Redirect,
    UpdateNavigationInput, UpdatePageInput,
};
pub use service::{
    ContentService, MAX_TRANSLATION_BYTES, MAX_TRANSLATION_RESOURCE_ID_LENGTH, PageValidationIssue,
    ResolvedTranslation, TRANSLATION_RESOURCE_TYPES, Translation, UpsertTranslationInput,
    resolve_translation,
};
