//! Universal storefront SSR runtime management.
//!
//! Phase 1 provides the process launcher and the HTTP gateway for workerd. It
//! deliberately does not know about FastCart artifacts or deployment records;
//! those arrive in later phases. The public API is small so the commerce server
//! does not depend on workerd configuration details.

pub mod artifact;
mod config;
mod error;
mod gateway;
mod process;
pub mod runtime;

pub use artifact::{Artifact, ArtifactError, ArtifactLimits, ArtifactManifest, ArtifactStore};
pub use config::WorkerdConfig;
pub use error::SsrError;
pub use gateway::{SsrBody, SsrGateway, SsrResponse};
pub use process::{RuntimeHealth, RuntimeStatus, WorkerdProcess};
pub use runtime::{RuntimeGroupId, RuntimeManager, RuntimeManagerConfig};
