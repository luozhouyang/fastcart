//! Shared foundational types for the Fastcart commerce platform.
//!
//! The kernel stays small: identifiers, primitive value objects, request context
//! types and the shared application error model. It must not depend on Axum,
//! SQLx or any infrastructure crate.

pub mod context;
pub mod error;
pub mod ids;
pub mod money;

pub use context::{RequestContext, StoreContext};
pub use error::{AppError, KernelError};
pub use ids::{CartId, CustomerId, OrderId, PaymentId, ProductId, StoreId, UserId, VariantId};
pub use money::{CurrencyCode, Money};
