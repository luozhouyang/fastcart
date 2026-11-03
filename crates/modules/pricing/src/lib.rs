//! The `pricing` commerce module.
//!
//! Owns price lists, rules, variant prices and the deterministic price
//! selector. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.

pub mod domain;
pub mod service;

pub use domain::{
    CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter, PriceList,
    PriceListRule, PriceListRuleInput, PriceListStatus, PriceListType, PriceSet, SelectedPrice,
    UpdatePriceInput, UpdatePriceListInput, select_price,
};
pub use service::PricingService;
