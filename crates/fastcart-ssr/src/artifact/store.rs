//! Validated, digest-addressed artifact storage.

use std::path::{Path, PathBuf};

use super::ArtifactError;
use super::digest::{ArtifactLimits, compute_tree_digest, io, measure_tree};
use super::manifest::{ArtifactManifest, validate_relative_path};

/// Name of the manifest at the artifact root.
pub const MANIFEST_FILE: &str = "fastcart-ssr.json";

/// One validated artifact on disk.
#[derive(Clone, Debug)]
pub struct Artifact {
    digest: String,
    root: PathBuf,
    manifest: ArtifactManifest,
}

impl Artifact {
    /// SHA-256 digest of the artifact tree.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Immutable artifact directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Parsed and validated manifest.
    #[must_use]
    pub fn manifest(&self) -> &ArtifactManifest {
        &self.manifest
    }
}

/// Content-addressed artifact storage rooted at a data directory.
#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    /// Creates a store rooted at `root`. The directory is created on import.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validates a source directory, computes its digest and copies it into an
    /// immutable directory named after the digest.
    ///
    /// Importing the same content twice is idempotent.
    pub fn import(&self, source: &Path, limits: ArtifactLimits) -> Result<Artifact, ArtifactError> {
        let manifest = read_manifest(source)?;
        validate_layout(source, &manifest, limits)?;
        let digest = compute_tree_digest(source)?;
        let destination = self.root.join(&digest);

        if destination.exists() {
            return self.load_with_limits(&digest, limits);
        }

        std::fs::create_dir_all(&self.root).map_err(|error| io(&self.root, error))?;
        let staging = self.staging_path();
        let result = (|| {
            copy_tree(source, &staging)?;
            if destination.exists() {
                std::fs::remove_dir_all(&staging).map_err(|error| io(&staging, error))?;
                return Ok(());
            }
            std::fs::rename(&staging, &destination).map_err(|error| io(&destination, error))
        })();
        if result.is_err() {
            std::fs::remove_dir_all(&staging).ok();
        }
        result?;
        self.load_with_limits(&digest, limits)
    }

    /// Loads an artifact by digest and verifies its content.
    pub fn load(&self, digest: &str) -> Result<Artifact, ArtifactError> {
        self.load_with_limits(digest, ArtifactLimits::default())
    }

    fn load_with_limits(
        &self,
        digest: &str,
        limits: ArtifactLimits,
    ) -> Result<Artifact, ArtifactError> {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ArtifactError::InvalidDigest(digest.to_owned()));
        }
        let root = self.root.join(digest);
        let manifest = read_manifest(&root)?;
        let actual = compute_tree_digest(&root)?;
        if !actual.eq_ignore_ascii_case(digest) {
            return Err(ArtifactError::DigestMismatch {
                expected: digest.to_owned(),
                actual,
            });
        }
        validate_layout(&root, &manifest, limits)?;
        Ok(Artifact {
            digest: digest.to_owned(),
            root,
            manifest,
        })
    }

    fn staging_path(&self) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        self.root
            .join(format!(".staging-{}-{nonce}", std::process::id()))
    }
}

/// Reads and validates the manifest at an artifact root.
pub fn read_manifest(root: &Path) -> Result<ArtifactManifest, ArtifactError> {
    let path = root.join(MANIFEST_FILE);
    if !path.is_file() {
        return Err(ArtifactError::MissingManifest(path));
    }
    let bytes = std::fs::read(&path).map_err(|error| io(&path, error))?;
    let manifest: ArtifactManifest = serde_json::from_slice(&bytes)
        .map_err(|error| ArtifactError::InvalidJson(error.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_layout(
    root: &Path,
    manifest: &ArtifactManifest,
    limits: ArtifactLimits,
) -> Result<(), ArtifactError> {
    validate_relative_path(&manifest.entry.path)?;
    let entry = root.join(&manifest.entry.path);
    let entry_meta = std::fs::symlink_metadata(&entry)
        .map_err(|_| ArtifactError::MissingEntry(manifest.entry.path.clone()))?;
    if entry_meta.file_type().is_symlink() || !entry_meta.is_file() {
        return Err(ArtifactError::MissingEntry(manifest.entry.path.clone()));
    }
    if !matches!(
        entry.extension().and_then(std::ffi::OsStr::to_str),
        Some("js" | "mjs")
    ) {
        return Err(ArtifactError::InvalidManifest(
            "entry.path must end in .js or .mjs".to_owned(),
        ));
    }

    validate_relative_path(&manifest.assets.directory)?;
    let assets = root.join(&manifest.assets.directory);
    let assets_meta = std::fs::symlink_metadata(&assets)
        .map_err(|_| ArtifactError::MissingAssets(manifest.assets.directory.clone()))?;
    if assets_meta.file_type().is_symlink() || !assets_meta.is_dir() {
        return Err(ArtifactError::MissingAssets(
            manifest.assets.directory.clone(),
        ));
    }

    let canonical_root = std::fs::canonicalize(root).map_err(|error| io(root, error))?;
    for candidate in [&entry, &assets] {
        let canonical = std::fs::canonicalize(candidate).map_err(|error| io(candidate, error))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(ArtifactError::UnsafePath(candidate.display().to_string()));
        }
    }
    measure_tree(root, limits)?;
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), ArtifactError> {
    std::fs::create_dir_all(destination).map_err(|error| io(destination, error))?;
    let mut entries: Vec<_> = std::fs::read_dir(source)
        .map_err(|error| io(source, error))?
        .map(|entry| entry.map_err(|error| io(source, error)))
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(ArtifactError::Symlink(path.display().to_string()));
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&path, &target)?;
        } else if metadata.is_file() {
            std::fs::copy(&path, &target).map_err(|error| io(&target, error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        AssetMetadata, BuildMetadata, EntryMetadata, FrameworkMetadata, RenderingMetadata,
        RuntimeMetadata,
    };

    fn manifest() -> ArtifactManifest {
        ArtifactManifest {
            schema_version: "1".to_owned(),
            artifact_id: "artifact_01".to_owned(),
            framework: FrameworkMetadata {
                name: "tanstack-start".to_owned(),
                version: "1.169.5".to_owned(),
            },
            runtime: RuntimeMetadata {
                engine: "workerd".to_owned(),
                compatibility_date: "2026-11-01".to_owned(),
                compatibility_flags: vec![],
                required_capabilities: vec!["fetch".to_owned()],
            },
            entry: EntryMetadata {
                entry_type: "worker".to_owned(),
                path: "worker/index.js".to_owned(),
                export: "default".to_owned(),
            },
            assets: AssetMetadata {
                directory: "client".to_owned(),
                base_path: "/_fastcart/assets/".to_owned(),
            },
            rendering: RenderingMetadata {
                mode: "hybrid".to_owned(),
                streaming: true,
            },
            build: BuildMetadata {
                build_id: "build_123".to_owned(),
                created_at: "2026-11-01T10:00:00Z".to_owned(),
            },
        }
    }

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("worker")).unwrap();
        std::fs::create_dir_all(directory.path().join("client/assets")).unwrap();
        std::fs::write(
            directory.path().join("worker/index.js"),
            b"export default { fetch() { return new Response('ok') } }",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("client/assets/app.js"),
            b"console.log(1)",
        )
        .unwrap();
        std::fs::write(
            directory.path().join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&manifest()).unwrap(),
        )
        .unwrap();
        directory
    }

    #[test]
    fn import_is_digest_addressed_and_idempotent() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(store_dir.path().join("artifacts"));
        let source = fixture();
        let first = store
            .import(source.path(), ArtifactLimits::default())
            .unwrap();
        assert_eq!(first.manifest().artifact_id, "artifact_01");
        assert_eq!(first.digest().len(), 64);
        assert!(first.root().join("worker/index.js").is_file());
        let second = store
            .import(source.path(), ArtifactLimits::default())
            .unwrap();
        assert_eq!(first.digest(), second.digest());
        let loaded = store.load(first.digest()).unwrap();
        assert_eq!(loaded.digest(), first.digest());
    }

    #[test]
    fn missing_entry_and_traversal_are_rejected() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(store_dir.path());
        let source = fixture();
        std::fs::remove_file(source.path().join("worker/index.js")).unwrap();
        assert!(matches!(
            store.import(source.path(), ArtifactLimits::default()),
            Err(ArtifactError::MissingEntry(_))
        ));

        let source = fixture();
        let mut value = manifest();
        value.entry.path = "../escape.js".to_owned();
        std::fs::write(
            source.path().join(MANIFEST_FILE),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            store.import(source.path(), ArtifactLimits::default()),
            Err(ArtifactError::UnsafePath(_))
        ));
    }

    #[test]
    fn tampered_stored_content_is_detected() {
        let store_dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(store_dir.path());
        let source = fixture();
        let artifact = store
            .import(source.path(), ArtifactLimits::default())
            .unwrap();
        let entry = artifact.root().join("worker/index.js");
        std::fs::write(&entry, b"tampered").unwrap();
        assert!(matches!(
            store.load(artifact.digest()),
            Err(ArtifactError::DigestMismatch { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_rejected() {
        let source = fixture();
        std::os::unix::fs::symlink(
            source.path().join("worker/index.js"),
            source.path().join("client/assets/link.js"),
        )
        .unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(store_dir.path());
        assert!(matches!(
            store.import(source.path(), ArtifactLimits::default()),
            Err(ArtifactError::Symlink(_))
        ));
    }
}
