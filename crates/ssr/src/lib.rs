//! FastCart universal storefront SSR service.
//!
//! This crate is the product layer: it decides *what* runs for a store and
//! *when*, and owns the deployment registry, activation and rollback, preview
//! deployments, the response cache and static asset serving.
//!
//! The workerd execution details (process supervision, the loopback gateway,
//! Cap'n Proto configuration and the artifact contract) live in
//! [`fastcart_ssr_workerd`]. Keeping the two apart means the deployment
//! lifecycle can be tested with a fake runtime, and workerd upgrades do not
//! touch the public service API.

mod config;
mod contract;
mod service;

pub use config::{SsrConfig, SsrCredentialsConfig, SsrDeploymentConfig};
pub use contract::UniversalStorefront;
pub use service::SsrService;
