//! The `checkout` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod service;

pub use domain::{
    CheckoutSession, CheckoutStateHistory, CheckoutStatus, CreateCheckoutInput,
    UpdateCheckoutDetailsInput,
};
pub use service::CheckoutService;
