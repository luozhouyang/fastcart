//! FastCart SSR artifact contract and immutable storage.

mod digest;
mod error;
mod manifest;
mod store;

pub use digest::{ArtifactLimits, compute_tree_digest};
pub use error::ArtifactError;
pub use manifest::{
    ArtifactManifest, AssetMetadata, BuildMetadata, EntryMetadata, FrameworkMetadata,
    RenderingMetadata, RuntimeMetadata,
};
pub use store::{Artifact, ArtifactStore, read_manifest};
