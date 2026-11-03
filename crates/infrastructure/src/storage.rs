//! Object storage abstraction with a local filesystem adapter.

use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;

use crate::{InfrastructureError, Result};

/// Metadata returned after storing an object.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StorageObject {
    /// Caller-provided storage key.
    pub key: String,
    /// Size in bytes.
    pub bytes: u64,
    /// Content type, when supplied.
    pub content_type: Option<String>,
}

/// Provider-independent object storage.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Stores or replaces an object.
    async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<StorageObject>;

    /// Reads an object.
    async fn get(&self, key: &str) -> Result<Vec<u8>>;

    /// Returns true when the object exists.
    async fn exists(&self, key: &str) -> Result<bool>;

    /// Deletes an object. Missing objects are a no-op.
    async fn delete(&self, key: &str) -> Result<()>;
}

/// Local filesystem storage rooted at a trusted directory.
#[derive(Clone, Debug)]
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    /// Creates storage rooted at the given directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the storage root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, key: &str) -> Result<PathBuf> {
        let relative = Path::new(key);
        if key.is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(InfrastructureError::InvalidInput(format!(
                "unsafe storage key: {key}"
            )));
        }
        Ok(self.root.join(relative))
    }
}

#[async_trait]
impl Storage for LocalStorage {
    async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<StorageObject> {
        let path = self.resolve(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, bytes).await?;
        Ok(StorageObject {
            key: key.to_owned(),
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            content_type: content_type.map(str::to_owned),
        })
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.resolve(key)?;
        Ok(tokio::fs::read(path).await?)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let path = self.resolve(key)?;
        Ok(tokio::fs::try_exists(path).await?)
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = self.resolve(key)?;
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalStorage, Storage};

    #[tokio::test]
    async fn stores_reads_and_deletes_objects() {
        let dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());

        storage
            .put("products/a.txt", b"hello", Some("text/plain"))
            .await
            .unwrap();
        assert!(storage.exists("products/a.txt").await.unwrap());
        assert_eq!(storage.get("products/a.txt").await.unwrap(), b"hello");

        storage.delete("products/a.txt").await.unwrap();
        assert!(!storage.exists("products/a.txt").await.unwrap());
        storage.delete("products/a.txt").await.unwrap();
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());

        assert!(storage.put("../escape.txt", b"x", None).await.is_err());
        assert!(storage.get("/etc/passwd").await.is_err());
        assert!(storage.get("a/../../b").await.is_err());
    }
}
