//! Content-addressed artifact store on the local filesystem.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::bundle::RuntimeBundle;
use crate::error::PluginError;
use crate::manifest::PluginManifest;

use super::archive::{ArchiveFile, ArchiveLimits, read_archive};
use super::digest::compute_digest;

const MANIFEST_FILE: &str = "plugin.json";

/// Summary of one stored artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ArtifactSummary {
    /// Artifact digest.
    pub digest: String,
    /// Plugin id from the manifest.
    pub id: String,
    /// Plugin name from the manifest.
    pub name: String,
    /// Plugin version from the manifest.
    pub version: String,
}

/// A stored artifact and its manifest.
#[derive(Clone, Debug)]
pub struct StoredArtifact {
    /// Artifact digest.
    pub digest: String,
    /// Parsed manifest.
    pub manifest: PluginManifest,
    /// Artifact directory.
    pub path: PathBuf,
}

/// An archive that passed validation but has not been stored yet.
#[derive(Clone, Debug)]
pub struct InspectedArtifact {
    /// Canonical digest of the archive content.
    pub digest: String,
    /// Validated manifest.
    pub manifest: PluginManifest,
}

/// Filesystem-backed artifact store.
#[derive(Clone, Debug)]
pub struct ArtifactStore {
    root: PathBuf,
    limits: ArchiveLimits,
    max_module_bytes: u64,
}

impl ArtifactStore {
    /// Creates a store rooted at `root`. Directories are created lazily.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, limits: ArchiveLimits, max_module_bytes: u64) -> Self {
        Self {
            root: root.into(),
            limits,
            max_module_bytes,
        }
    }

    /// Returns the artifact root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts")
    }

    fn artifact_dir(&self, digest: &str) -> PathBuf {
        self.artifacts_dir().join(digest)
    }

    /// Validates an archive without writing it.
    ///
    /// # Errors
    ///
    /// Returns validation and unsafe-path errors.
    pub fn inspect(&self, bytes: &[u8]) -> Result<InspectedArtifact, PluginError> {
        let (_, manifest, digest) = self.prepare(bytes)?;
        Ok(InspectedArtifact { digest, manifest })
    }

    fn prepare(
        &self,
        bytes: &[u8],
    ) -> Result<(super::archive::Archive, PluginManifest, String), PluginError> {
        let archive = read_archive(bytes, &self.limits)?;
        let manifest_file = archive.file(MANIFEST_FILE).ok_or_else(|| {
            PluginError::InvalidArchive("archive is missing plugin.json at its root".to_owned())
        })?;
        let manifest = PluginManifest::parse(&manifest_file.bytes)?;
        manifest.validate()?;
        let sizes: BTreeMap<String, u64> = archive
            .files
            .iter()
            .map(|file| (file.path.clone(), file.bytes.len() as u64))
            .collect();
        manifest.validate_archive(&sizes, self.max_module_bytes)?;
        let digest = compute_digest(&archive.files);
        Ok((archive, manifest, digest))
    }

    /// Installs an archive and returns the stored artifact.
    ///
    /// Installing the same content twice is idempotent: the existing directory
    /// is reused after its digest is verified.
    ///
    /// # Errors
    ///
    /// Returns validation errors, unsafe-path errors and storage errors.
    pub fn install(&self, bytes: &[u8]) -> Result<StoredArtifact, PluginError> {
        let (archive, _manifest, digest) = self.prepare(bytes)?;
        let target = self.artifact_dir(&digest);
        if target.is_dir() {
            return self.load(&digest);
        }

        let tmp_root = self.root.join("tmp");
        fs::create_dir_all(&tmp_root).map_err(|error| PluginError::Storage(error.to_string()))?;
        let temp = tmp_root.join(format!("install-{}", Uuid::now_v7().simple()));
        fs::create_dir_all(&temp).map_err(|error| PluginError::Storage(error.to_string()))?;

        let result = write_archive(&temp, &archive.files)
            .and_then(|()| {
                fs::create_dir_all(self.artifacts_dir())
                    .map_err(|error| PluginError::Storage(error.to_string()))
            })
            .and_then(|()| match fs::rename(&temp, &target) {
                Ok(()) => Ok(()),
                Err(error) if target.is_dir() => {
                    // Another process installed the same digest first.
                    let _ = fs::remove_dir_all(&temp);
                    let _ = error;
                    Ok(())
                }
                Err(error) => Err(PluginError::Storage(error.to_string())),
            });
        if result.is_err() {
            let _ = fs::remove_dir_all(&temp);
        }
        result?;

        self.load(&digest)
    }

    /// Deletes artifacts whose digest is not referenced.
    ///
    /// Artifacts newer than `min_age` are kept to avoid racing an in-flight
    /// install that has not committed its database row yet. Callers must gather
    /// references from every store before invoking this.
    ///
    /// # Errors
    ///
    /// Returns storage errors.
    pub fn prune(
        &self,
        referenced: &HashSet<String>,
        min_age: std::time::Duration,
    ) -> Result<Vec<String>, PluginError> {
        let dir = self.artifacts_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|error| PluginError::Storage(error.to_string()))? {
            let entry = entry.map_err(|error| PluginError::Storage(error.to_string()))?;
            let metadata = entry
                .metadata()
                .map_err(|error| PluginError::Storage(error.to_string()))?;
            if !metadata.is_dir() {
                continue;
            }
            if metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age < min_age)
            {
                continue;
            }
            let digest = entry.file_name().to_string_lossy().to_string();
            if referenced.contains(&digest) {
                continue;
            }
            fs::remove_dir_all(entry.path())
                .map_err(|error| PluginError::Storage(error.to_string()))?;
            removed.push(digest);
        }
        removed.sort();
        Ok(removed)
    }

    /// Reads one file from an installed artifact.
    ///
    /// The relative path is rejected when it is absolute, escapes the artifact
    /// directory, or contains a non-normal component.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::UnsafePath`] for unsafe paths and
    /// [`PluginError::NotFound`] when the file is absent.
    pub fn read_file(&self, digest: &str, relative: &str) -> Result<Vec<u8>, PluginError> {
        if relative.is_empty() || relative.starts_with('/') || relative.contains('\\') {
            return Err(PluginError::UnsafePath(relative.to_owned()));
        }
        if Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(PluginError::UnsafePath(relative.to_owned()));
        }
        let path = self.artifact_dir(digest).join(relative);
        if !path.is_file() {
            return Err(PluginError::NotFound(format!("{digest}/{relative}")));
        }
        std::fs::read(&path).map_err(|error| PluginError::Storage(error.to_string()))
    }

    /// Reads the manifest of a stored artifact for a listing.
    ///
    /// The directory name is the digest assigned when the artifact was
    /// imported, and its contents were verified then. A listing therefore
    /// reads `plugin.json` instead of re-hashing every file; a missing or
    /// unreadable manifest falls back to full verification.
    fn summary_manifest(&self, digest: &str) -> Result<PluginManifest, PluginError> {
        let path = self.artifact_dir(digest).join(MANIFEST_FILE);
        match fs::read(&path) {
            Ok(bytes) => PluginManifest::parse(&bytes),
            Err(_) => self.load(digest).map(|stored| stored.manifest),
        }
    }

    /// Loads an artifact by digest and verifies its content.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] when absent and
    /// [`PluginError::DigestMismatch`] when the stored content was modified.
    pub fn load(&self, digest: &str) -> Result<StoredArtifact, PluginError> {
        let path = self.artifact_dir(digest);
        if !path.is_dir() {
            return Err(PluginError::NotFound(digest.to_owned()));
        }
        let files = read_dir_files(&path)?;
        let actual = compute_digest(&files);
        if actual != digest {
            return Err(PluginError::DigestMismatch(format!(
                "expected {digest}, found {actual}"
            )));
        }
        let manifest_file = files
            .iter()
            .find(|file| file.path == MANIFEST_FILE)
            .ok_or_else(|| PluginError::DigestMismatch("missing plugin.json".to_owned()))?;
        let manifest = PluginManifest::parse(&manifest_file.bytes)?;
        manifest.validate()?;
        Ok(StoredArtifact {
            digest: digest.to_owned(),
            manifest,
            path,
        })
    }

    /// Builds the runtime bundle for one digest.
    ///
    /// # Errors
    ///
    /// Returns storage and UTF-8 errors.
    pub fn read_bundle(&self, digest: &str) -> Result<RuntimeBundle, PluginError> {
        let artifact = self.load(digest)?;
        let files = read_dir_files(&artifact.path)?;
        let mut modules = BTreeMap::new();
        for file in &files {
            if file.path.ends_with(".js") || file.path.ends_with(".mjs") {
                let source = String::from_utf8(file.bytes.clone()).map_err(|_| {
                    PluginError::Storage(format!("module is not UTF-8: {}", file.path))
                })?;
                modules.insert(file.path.clone(), source);
            }
        }
        if modules.is_empty() {
            return Err(PluginError::InvalidArchive(
                "artifact contains no JavaScript modules".to_owned(),
            ));
        }
        Ok(RuntimeBundle::new(
            artifact.digest,
            &artifact.manifest,
            modules,
        ))
    }

    /// Lists stored artifacts, newest first by digest.
    ///
    /// # Errors
    ///
    /// Storage errors are returned; unreadable artifacts are skipped.
    pub fn list(&self) -> Result<Vec<ArtifactSummary>, PluginError> {
        let dir = self.artifacts_dir();
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let entries =
            fs::read_dir(&dir).map_err(|error| PluginError::Storage(error.to_string()))?;
        let mut summaries = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| PluginError::Storage(error.to_string()))?;
            if !entry
                .file_type()
                .map_err(|error| PluginError::Storage(error.to_string()))?
                .is_dir()
            {
                continue;
            }
            let digest = entry.file_name().to_string_lossy().to_string();
            let manifest = match self.summary_manifest(&digest) {
                Ok(manifest) => manifest,
                Err(error) => {
                    tracing::warn!(digest = %digest, error = %error, "skipping unreadable plugin artifact");
                    continue;
                }
            };
            summaries.push(ArtifactSummary {
                digest,
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
            });
        }
        summaries.sort_by(|left, right| left.digest.cmp(&right.digest));
        Ok(summaries)
    }
}

fn write_archive(root: &Path, files: &[ArchiveFile]) -> Result<(), PluginError> {
    for file in files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| PluginError::Storage(error.to_string()))?;
        }
        fs::write(&path, &file.bytes).map_err(|error| PluginError::Storage(error.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o444));
        }
    }
    Ok(())
}

fn read_dir_files(root: &Path) -> Result<Vec<ArchiveFile>, PluginError> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<ArchiveFile>) -> Result<(), PluginError> {
    for entry in fs::read_dir(directory).map_err(|error| PluginError::Storage(error.to_string()))? {
        let entry = entry.map_err(|error| PluginError::Storage(error.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|error| PluginError::Storage(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginError::UnsafePath(format!(
                "symlink in artifact: {}",
                entry.path().display()
            )));
        }
        if metadata.is_dir() {
            walk(root, &entry.path(), files)?;
        } else if metadata.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| PluginError::Storage("artifact path escape".to_owned()))?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes =
                fs::read(entry.path()).map_err(|error| PluginError::Storage(error.to_string()))?;
            files.push(ArchiveFile {
                path: relative,
                bytes,
            });
        }
    }
    Ok(())
}
