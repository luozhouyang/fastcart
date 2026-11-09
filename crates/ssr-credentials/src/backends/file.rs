//! Atomic, locked JSON file backend.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead as _, AeadCore as _, OsRng};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};
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

#[derive(Serialize, Deserialize)]
struct EncryptedCredentials {
    version: u32,
    algorithm: String,
    nonce: String,
    ciphertext: String,
}

/// A credential file stored with atomic replacement and a `0600` mode.
#[derive(Clone)]
pub struct FileCredentialBackend {
    path: PathBuf,
    read_only: bool,
    key: Option<[u8; 32]>,
}

impl std::fmt::Debug for FileCredentialBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileCredentialBackend")
            .field("path", &self.path)
            .field("read_only", &self.read_only)
            .field("encrypted", &self.key.is_some())
            .finish()
    }
}

impl FileCredentialBackend {
    /// Creates a file backend at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            path: path.into(),
            read_only,
            key: None,
        }
    }

    /// Creates an encrypted file backend using a 32-byte XChaCha20-Poly1305 key.
    #[must_use]
    pub fn with_key(path: impl Into<PathBuf>, read_only: bool, key: [u8; 32]) -> Self {
        Self {
            path: path.into(),
            read_only,
            key: Some(key),
        }
    }

    /// Whether writes are encrypted.
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.key.is_some()
    }

    /// Decodes a base64 32-byte XChaCha20-Poly1305 key.
    pub fn key_from_base64(value: &str) -> Result<[u8; 32], BackendError> {
        let bytes = BASE64
            .decode(value.trim())
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
        bytes
            .try_into()
            .map_err(|_| BackendError::Invalid("credential key must be 32 bytes".to_owned()))
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
        let persisted = self.decode(&bytes)?;
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

    fn decode(&self, bytes: &[u8]) -> Result<PersistedCredentials, BackendError> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => serde_json::from_value(value)
                .map_err(|error| BackendError::Invalid(error.to_string())),
            Some(2) => {
                let key = self.key.ok_or_else(|| {
                    BackendError::Invalid("credential key is not configured".to_owned())
                })?;
                let envelope: EncryptedCredentials = serde_json::from_value(value)
                    .map_err(|error| BackendError::Invalid(error.to_string()))?;
                if envelope.algorithm != "xchacha20poly1305" {
                    return Err(BackendError::Invalid(format!(
                        "unsupported credential encryption {}",
                        envelope.algorithm
                    )));
                }
                let nonce = BASE64
                    .decode(envelope.nonce)
                    .map_err(|error| BackendError::Invalid(error.to_string()))?;
                let nonce: [u8; 24] = nonce
                    .try_into()
                    .map_err(|_| BackendError::Invalid("invalid credential nonce".to_owned()))?;
                let ciphertext = BASE64
                    .decode(envelope.ciphertext)
                    .map_err(|error| BackendError::Invalid(error.to_string()))?;
                let cipher = XChaCha20Poly1305::new((&key).into());
                let plaintext = cipher
                    .decrypt(XNonce::from_slice(&nonce), ciphertext.as_ref())
                    .map_err(|_| {
                        BackendError::Invalid("credential decryption failed".to_owned())
                    })?;
                serde_json::from_slice(&plaintext)
                    .map_err(|error| BackendError::Invalid(error.to_string()))
            }
            Some(version) => Err(BackendError::Invalid(format!(
                "unsupported credential version {version}"
            ))),
            None => Err(BackendError::Invalid(
                "credential version is missing".to_owned(),
            )),
        }
    }

    fn encode(&self, tokens: &[CredentialToken]) -> Result<Vec<u8>, BackendError> {
        let persisted = PersistedCredentials {
            version: 1,
            tokens: tokens.to_vec(),
        };
        let plaintext = serde_json::to_vec(&persisted)
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
        let Some(key) = self.key else {
            let mut bytes = serde_json::to_vec_pretty(&persisted)
                .map_err(|error| BackendError::Invalid(error.to_string()))?;
            bytes.push(b'\n');
            return Ok(bytes);
        };
        let cipher = XChaCha20Poly1305::new((&key).into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_ref())
            .map_err(|_| BackendError::Invalid("credential encryption failed".to_owned()))?;
        let envelope = EncryptedCredentials {
            version: 2,
            algorithm: "xchacha20poly1305".to_owned(),
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        };
        let mut bytes = serde_json::to_vec_pretty(&envelope)
            .map_err(|error| BackendError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        Ok(bytes)
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
            let bytes = self.encode(&snapshot.tokens)?;
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
