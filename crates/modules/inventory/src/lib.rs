//! The `inventory` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod service;

pub use domain::*;
pub use service::*;

#[cfg(test)]
mod tests;
