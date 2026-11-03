//! Platform-level domain: store registry, domains, lifecycle and users.
//!
//! This crate is independent of HTTP, SQLx and the storefront runtime. Concrete
//! persistence implementations belong in `fastcart-infrastructure`.

pub mod domain;
pub mod error;
pub mod store;

pub use domain::{StoreDomain, normalize_hostname};
pub use error::PlatformError;
pub use store::{CreateStoreInput, Store, StoreRegistry, StoreStatus};
