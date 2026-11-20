//! The `cart` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.
//!
//! The service stores price and adjustment snapshots supplied by upstream
//! workflows. It never calls the pricing, promotion, tax or shipping modules and
//! never accepts a client-computed total: [`calculate_totals`] always derives the
//! commercial summary from the persisted rows.

mod domain;
mod service;

pub use domain::{
    AddItemInput, Address, AddressInput, AdjustmentInput, Cart, CartAdjustment, CartDiscount,
    CartItem, CartShippingMethod, CartStatus, CartTaxLine, CartTotals, CreateCartInput,
    DEFAULT_CART_TTL_SECONDS, DiscountInput, ItemPriceInput, MAX_ITEM_QUANTITY, RecalculationInput,
    SetShippingMethodInput, TaxLineInput, UpdateItemInput, allocate_cart_discounts,
    allocate_cart_level_discount, calculate_totals, recalculate_totals,
};
pub use service::CartService;
