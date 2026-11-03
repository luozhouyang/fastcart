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

    async fn safe_existing(&self, key: &str) -> Result<PathBuf> {
        let root = tokio::fs::canonicalize(&self.root).await?;
        let path = tokio::fs::canonicalize(self.resolve(key)?).await?;
        if !path.starts_with(&root) {
            return Err(InfrastructureError::InvalidInput(format!(
                "storage key escapes the root: {key}"
            )));
        }
        Ok(path)
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
        tokio::fs::create_dir_all(&self.root).await?;
        let root = tokio::fs::canonicalize(&self.root).await?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
            let canonical_parent = tokio::fs::canonicalize(parent).await?;
            if !canonical_parent.starts_with(&root) {
                return Err(InfrastructureError::InvalidInput(format!(
                    "storage key escapes the root: {key}"
                )));
            }
        }
        if tokio::fs::symlink_metadata(&path)
            .await
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(InfrastructureError::InvalidInput(format!(
                "storage key points at a symbolic link: {key}"
            )));
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                InfrastructureError::InvalidInput("storage key has no file name".to_owned())
            })?;
        let temporary = path.with_file_name(format!(
            ".{file_name}.{}.tmp",
            uuid::Uuid::now_v7().simple()
        ));
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, &path).await?;
        Ok(StorageObject {
            key: key.to_owned(),
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            content_type: content_type.map(str::to_owned),
        })
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>> {
        let path = self.safe_existing(key).await?;
        Ok(tokio::fs::read(path).await?)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        match self.safe_existing(key).await {
            Ok(path) => Ok(tokio::fs::try_exists(path).await?),
            Err(InfrastructureError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let path = match self.safe_existing(key).await {
            Ok(path) => path,
            Err(InfrastructureError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
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
    async fn rejects_symbolic_link_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let storage = LocalStorage::new(dir.path());
        std::fs::create_dir_all(dir.path().join("nested")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.path().join("nested/link")).unwrap();

        assert!(
            storage
                .put("nested/link/escape.txt", b"x", None)
                .await
                .is_err()
        );
        assert!(storage.get("nested/link/escape.txt").await.is_err());
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
