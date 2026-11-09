//! workerd execution layer for the universal storefront runtime.
//!
//! This crate owns everything that is specific to running an artifact inside a
//! pinned `workerd` process: process supervision, the loopback HTTP gateway,
//! Cap'n Proto config generation, runtime groups and the `fastcart-ssr.json`
//! artifact contract.
//!
//! Deployment lifecycle, activation, previews, response caching and static
//! assets belong to `fastcart-ssr`, which builds on this crate. Nothing here
//! knows about stores or deployment records, so the commerce server never
//! depends on workerd configuration details.

pub mod artifact;
mod config;
mod error;
mod gateway;
mod process;
pub mod runtime;

pub use artifact::{Artifact, ArtifactError, ArtifactLimits, ArtifactManifest, ArtifactStore};
pub use config::WorkerdConfig;
pub use error::SsrError;
pub use gateway::{SsrBody, SsrGateway, SsrRequestContext, SsrResponse};
pub use process::{RuntimeHealth, RuntimeStatus, WorkerdProcess};
pub use runtime::{RuntimeGroupId, RuntimeManager, RuntimeManagerConfig};
