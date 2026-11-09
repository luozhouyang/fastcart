//! Pluggable storage for store-scoped internal SSR credentials.
//!
//! The file backend is the only concrete backend in this phase. Additional
//! secret-manager adapters can implement [`CredentialBackend`] without changing
//! the API or gateway.

mod backend;
pub mod backends;
mod manager;
mod model;

pub use backend::{BackendError, CredentialBackend};
pub use backends::file::FileCredentialBackend;
pub use manager::{CredentialError, CredentialManager, CredentialMode};
pub use model::{CredentialSnapshot, CredentialToken};
