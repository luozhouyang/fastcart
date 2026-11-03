//! The `store` commerce module.
//!
//! Owns store-level configuration (the `settings` table) and sales channels.
//! The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod service;

#[cfg(test)]
mod tests;

pub use domain::{CreateSalesChannelInput, SalesChannel, UpdateSalesChannelInput};
pub use service::{
    MAX_SALES_CHANNEL_NAME_LENGTH, MAX_SETTING_KEY_LENGTH, MAX_SETTING_VALUE_BYTES, StoreService,
};
