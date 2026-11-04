//! The `content` commerce module.
//!
//! Domain rules, application services and SQLite repositories for pages, page
//! versions, navigation and redirects live here. The crate is self-contained:
//! cross-module orchestration belongs in `fastcart-workflows`.

mod domain;
mod service;

#[cfg(test)]
mod tests;

pub use domain::{
    CreateNavigationInput, CreatePageInput, CreateRedirectInput, MAX_BODY_BYTES, Navigation,
    NavigationItem, NavigationItemInput, NavigationTree, NavigationTreeItem, Page,
    PageRevisionOrigin, PageStatus, PageVersion, PageVersionKind, Redirect, UpdateNavigationInput,
    UpdatePageInput,
};
pub use service::ContentService;
