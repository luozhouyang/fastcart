//! Commerce domain modules.
//!
//! Business rules, entities, application services and repository contracts live
//! here. Domain code must not depend on Axum, SQLx, SQLite or provider SDKs.
//!
//! Each module is intentionally empty during Phase 0. Phase 5 onwards expands
//! them following the layout `domain/`, `application/` and `ports/`.

/// Products, variants, options, categories, collections and media.
pub mod catalog {}

/// Price lists, currencies and centralized price calculation.
pub mod pricing {}

/// Stock locations, levels, movements and reservations.
pub mod inventory {}

/// Customers, addresses, groups and store-scoped identity.
pub mod customer {}

/// Carts and cart items with server-calculated commercial totals.
pub mod cart {}

/// Checkout orchestration and persisted checkout sessions.
pub mod checkout {}

/// Orders, snapshots, status history and fulfillment.
pub mod order {}

/// Provider-independent payment sessions, attempts and refunds.
pub mod payment {}

/// Promotions, coupons, rules and usage accounting.
pub mod promotion {}

/// Shipping profiles, options, rates and fulfillment.
pub mod shipping {}

/// Tax categories, regions, rates and calculation.
pub mod tax {}

/// Pages, navigation, redirects and SEO metadata.
pub mod content {}
