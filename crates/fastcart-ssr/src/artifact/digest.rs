//! Deterministic artifact digests and size limits.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::ArtifactError;

/// Limits enforced before an artifact is copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactLimits {
    /// Maximum total bytes in the artifact tree.
    pub max_bytes: u64,
    /// Maximum number of files in the artifact tree.
    pub max_files: u64,
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
            max_files: 5_000,
        }
    }
}

/// Computes the SHA-256 digest over sorted relative paths and file contents.
pub fn compute_tree_digest(root: &Path) -> Result<String, ArtifactError> {
    let files = collect_files(root)?;
    let mut hasher = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let mut file = std::fs::File::open(&path).map_err(|error| io(&path, error))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| io(&path, error))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Measures a tree and rejects it when a limit is exceeded.
pub(crate) fn measure_tree(
    root: &Path,
    limits: ArtifactLimits,
) -> Result<(u64, u64), ArtifactError> {
    let files = collect_files(root)?;
    let file_count = u64::try_from(files.len()).unwrap_or(u64::MAX);
    if file_count > limits.max_files {
        return Err(ArtifactError::ExceedsLimit {
            kind: "files",
            actual: file_count,
            limit: limits.max_files,
        });
    }
    let mut total = 0_u64;
    for relative in &files {
        let path = root.join(relative);
        let metadata = std::fs::metadata(&path).map_err(|error| io(&path, error))?;
        total = total.saturating_add(metadata.len());
        if total > limits.max_bytes {
            return Err(ArtifactError::ExceedsLimit {
                kind: "bytes",
                actual: total,
                limit: limits.max_bytes,
            });
        }
    }
    Ok((total, file_count))
}

fn collect_files(root: &Path) -> Result<Vec<String>, ArtifactError> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<(), ArtifactError> {
    let entries = std::fs::read_dir(directory).map_err(|error| io(directory, error))?;
    let mut entries: Vec<_> = entries
        .map(|entry| entry.map_err(|error| io(directory, error)))
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactError::Symlink(path.display().to_string()));
        }
        if metadata.is_dir() {
            walk(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ArtifactError::UnsafePath(path.display().to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}

pub(crate) fn io(path: &Path, error: std::io::Error) -> ArtifactError {
    ArtifactError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_order_independent_and_content_sensitive() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("a")).unwrap();
        std::fs::write(directory.path().join("b.js"), b"b").unwrap();
        std::fs::write(directory.path().join("a/c.js"), b"c").unwrap();
        let first = compute_tree_digest(directory.path()).unwrap();
        let second = compute_tree_digest(directory.path()).unwrap();
        assert_eq!(first, second);
        std::fs::write(directory.path().join("b.js"), b"changed").unwrap();
        assert_ne!(first, compute_tree_digest(directory.path()).unwrap());
    }

    #[test]
    fn digest_matches_the_typescript_packager_algorithm() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.js"), b"a").unwrap();
        std::fs::create_dir(directory.path().join("dir")).unwrap();
        std::fs::write(directory.path().join("dir/b.js"), b"b").unwrap();
        assert_eq!(
            compute_tree_digest(directory.path()).unwrap(),
            "2f7c7d75e1f26542aa87b4547891d398475450e46cc7186c40de981b12e0e10b"
        );
    }

    #[test]
    fn limits_reject_large_trees() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("large.js"), b"0123456789").unwrap();
        let error = measure_tree(
            directory.path(),
            ArtifactLimits {
                max_bytes: 5,
                max_files: 10,
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ArtifactError::ExceedsLimit { kind: "bytes", .. }
        ));
    }
}
