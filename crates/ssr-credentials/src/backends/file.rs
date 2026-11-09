//! Atomic, locked JSON file backend.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::{BackendError, CredentialBackend};
use crate::model::{CredentialSnapshot, CredentialToken};

#[derive(Serialize, Deserialize)]
struct PersistedCredentials {
    version: u32,
    tokens: Vec<CredentialToken>,
}

/// A credential file stored with atomic replacement and a `0600` mode.
#[derive(Clone, Debug)]
pub struct FileCredentialBackend {
    path: PathBuf,
    read_only: bool,
}

impl FileCredentialBackend {
    /// Creates a file backend at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            path: path.into(),
            read_only,
        }
    }

    /// Backing file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn revision(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    fn read_snapshot(&self) -> Result<Option<CredentialSnapshot>, BackendError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(BackendError::Io(error)),
        };
        let persisted: PersistedCredentials = serde_json::from_slice(&bytes)
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
        if persisted.version != 1 {
            return Err(BackendError::Invalid(format!(
                "unsupported credential version {}",
                persisted.version
            )));
        }
        Ok(Some(CredentialSnapshot {
            revision: Some(Self::revision(&bytes)),
            tokens: persisted.tokens,
        }))
    }
}

#[async_trait]
impl CredentialBackend for FileCredentialBackend {
    async fn load(&self) -> Result<Option<CredentialSnapshot>, BackendError> {
        self.read_snapshot()
    }

    async fn save(
        &self,
        snapshot: &CredentialSnapshot,
        expected_revision: Option<&str>,
    ) -> Result<String, BackendError> {
        if self.read_only {
            return Err(BackendError::ReadOnly);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock_path = self.path.with_extension("json.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock.lock_exclusive()?;

        let result = (|| {
            let current = self.read_snapshot()?;
            let current_revision = current.as_ref().and_then(|value| value.revision.as_deref());
            if current_revision != expected_revision {
                return Err(BackendError::Conflict);
            }
            let mut bytes = serde_json::to_vec_pretty(&PersistedCredentials {
                version: 1,
                tokens: snapshot.tokens.clone(),
            })
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
            bytes.push(b'\n');
            let staging = self.path.with_extension("json.tmp");
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&staging)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&staging, &self.path)?;
            Ok(Self::revision(&bytes))
        })();

        let _ = FileExt::unlock(&lock);
        result
    }
}
