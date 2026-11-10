//! Content-addressed artifact digests.

use sha2::{Digest, Sha256};

use super::archive::ArchiveFile;

const DOMAIN: &[u8] = b"fastcart-plugin-artifact-v1\0";

/// Computes the canonical digest of an extracted artifact.
///
/// Files are sorted by path so the digest is independent of archive order.
#[must_use]
pub fn compute_digest(files: &[ArchiveFile]) -> String {
    let mut sorted: Vec<&ArchiveFile> = files.iter().collect();
    sorted.sort_by(|left, right| left.path.cmp(&right.path));

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    for file in sorted {
        hasher.update(b"file\0");
        hasher.update(file.path.as_bytes());
        hasher.update(b"\0");
        hasher.update((file.bytes.len() as u64).to_be_bytes());
        hasher.update(&file.bytes);
    }
    hex::encode(hasher.finalize())
}
