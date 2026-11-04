//! The `store` commerce module.
//!
//! Owns store-level configuration (the `settings` table) and sales channels.
//! The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod region;
pub mod service;

#[cfg(test)]
mod tests;

pub use domain::{CreateSalesChannelInput, SalesChannel, UpdateSalesChannelInput};
pub use region::{
    CreatePublishableApiKeyInput, CreateRegionInput, CreatedPublishableApiKey, PublishableApiKey,
    Region, RegionService, UpdateRegionInput,
};
pub use service::{
    MAX_SALES_CHANNEL_NAME_LENGTH, MAX_SETTING_KEY_LENGTH, MAX_SETTING_VALUE_BYTES, StoreService,
    is_secret_setting_key,
};
