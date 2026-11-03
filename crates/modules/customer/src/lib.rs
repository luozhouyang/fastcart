//! The `customer` commerce module.
//!
//! Domain rules, application services and SQLite repositories for this module
//! live here. The crate is self-contained: cross-module orchestration belongs in
//! `fastcart-workflows`.
//!
//! The module owns customers, addresses, customer groups, credentials and
//! store-scoped customer sessions. Credentials are hashed with Argon2id and
//! session tokens are persisted only as SHA-256 digests.

pub mod domain;
pub mod service;

pub use domain::{
    Address, AddressInput, AuthenticatedCustomerSession, CUSTOMER_SESSION_TTL_SECONDS,
    CreateCustomerInput, CreatedCustomerSession, Customer, CustomerGroup, CustomerSession,
    CustomerStatus, LoginInput, MAX_NAME_LENGTH, MAX_PHONE_LENGTH, MIN_PASSWORD_LENGTH,
    RegisterCustomerInput, UpdateCustomerInput,
};
pub use service::CustomerService;
