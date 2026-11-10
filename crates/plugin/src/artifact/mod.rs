//! Plugin artifact validation, digesting and storage.

pub mod archive;
pub mod digest;
pub mod store;

pub use archive::{Archive, ArchiveFile, ArchiveLimits, read_archive};
pub use digest::compute_digest;
pub use store::{ArtifactStore, ArtifactSummary, InspectedArtifact, StoredArtifact};
