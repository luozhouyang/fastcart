//! Controlled plugin manifests, artifacts, per-store installation state and the
//! supervised workerd host.
//!
//! The crate validates manifests and archives, stores content-addressed
//! artifacts, records installation state, and (since P2) generates and
//! supervises the shared workerd process that executes enabled plugins.

pub mod artifact;
pub mod bundle;
pub mod capability;
pub mod dispatch;
pub mod error;
pub mod inject;
pub mod manifest;
pub mod model;
pub mod runtime;
pub mod service;
pub mod targets;

pub use artifact::{ArchiveLimits, ArtifactStore, ArtifactSummary, StoredArtifact};
pub use bundle::RuntimeBundle;
pub use capability::Scope;
pub use dispatch::{DispatchReport, PluginDispatch};
pub use error::PluginError;
pub use manifest::{HOST_API_VERSION, PluginManifest};
pub use model::{InstalledPlugin, PluginDetail};
pub use runtime::{
    ContextClaims, EgressError, EgressLimiter, EgressRequest, EgressResponse, PluginHost,
    RequestKind, RuntimeConfig, RuntimeRequest, RuntimeResponse, StreamingResponse, TokenActor,
    TokenSigner,
};
pub use service::PluginService;
