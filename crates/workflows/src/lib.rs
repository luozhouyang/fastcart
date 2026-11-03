//! Cross-module commerce workflows.
//!
//! Workflows orchestrate isolated module services. They own transaction and
//! idempotency boundaries and never contain provider-specific HTTP code.

pub mod cart;
pub mod checkout;
pub mod maintenance;

pub use cart::{CartContext, CartWorkflow};
pub use checkout::{CheckoutWorkflow, CompleteCheckoutResult, CreateCheckoutRequest};
pub use maintenance::{MaintenanceReport, run_maintenance};
