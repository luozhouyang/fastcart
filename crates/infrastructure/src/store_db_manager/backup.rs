//! Backup, verification and restore.

use super::*;

impl StoreDatabaseManager {
    /// Creates a consistent SQLite backup without copying an active WAL file.
    ///
    /// `VACUUM INTO` produces a transactionally consistent snapshot regardless
    /// of concurrent readers and writers. A sibling `<backup>.manifest.json`
    /// manifest is written after the checksum is computed.
    pub async fn backup(
        &self,
        store_id: &StoreId,
        destination: impl AsRef<Path>,
    ) -> Result<BackupManifest> {
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent).await?;
        }
        if destination.exists() {
            tokio::fs::remove_file(destination).await?;
        }

        let database = self.database(store_id).await?;
        sqlx::query("VACUUM INTO ?")
            .bind(destination.to_string_lossy().as_ref())
            .execute(database.pool())
            .await?;

        let (bytes, sha256) = Self::sha256_file(destination).await?;
        let manifest = BackupManifest {
            store_id: store_id.to_string(),
            created_at: now_timestamp(),
            file: destination.to_string_lossy().into_owned(),
            bytes,
            sha256,
        };
        let manifest_path = Self::manifest_path(destination);
        let encoded = serde_json::to_vec_pretty(&manifest).map_err(|error| {
            InfrastructureError::InvalidInput(format!(
                "failed to serialize backup manifest: {error}"
            ))
        })?;
        tokio::fs::write(&manifest_path, encoded).await?;
        Ok(manifest)
    }
}

impl StoreDatabaseManager {
    /// Returns the sibling manifest path for a backup file.
    ///
    /// The manifest is appended to the file name, so a backup at
    /// `store.sqlite` has a manifest at `store.sqlite.manifest.json`.
    #[must_use]
    pub fn manifest_path(backup: impl AsRef<Path>) -> PathBuf {
        let mut path = backup.as_ref().as_os_str().to_owned();
        path.push(".manifest.json");
        PathBuf::from(path)
    }
}

impl StoreDatabaseManager {
    /// Loads the sibling manifest for a backup file, when one exists.
    pub async fn load_backup_manifest(backup: impl AsRef<Path>) -> Result<Option<BackupManifest>> {
        let manifest_path = Self::manifest_path(backup);
        match tokio::fs::read(&manifest_path).await {
            Ok(bytes) => {
                let manifest = serde_json::from_slice(&bytes).map_err(|error| {
                    InfrastructureError::InvalidInput(format!(
                        "invalid backup manifest {}: {error}",
                        manifest_path.display()
                    ))
                })?;
                Ok(Some(manifest))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

impl StoreDatabaseManager {
    /// Verifies a backup file without creating or modifying it.
    ///
    /// The file is opened through a read-only SQLite connection. No
    /// create-if-missing, WAL or journal pragmas are applied, so verification
    /// never creates the database or `-wal`/`-shm` sidecars.
    pub async fn verify_backup(path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let metadata = tokio::fs::metadata(path).await.map_err(|error| {
            InfrastructureError::InvalidInput(format!("backup is not readable: {error}"))
        })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(InfrastructureError::Integrity(
                "backup file is empty or not a regular file".to_owned(),
            ));
        }
        let connect_options = SqliteConnectOptions::from_str(&path.to_string_lossy())
            .map_err(|error| InfrastructureError::InvalidInput(error.to_string()))?
            .read_only(true)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options)
            .await?;
        let result = async {
            verify_integrity(&pool).await?;
            let objects: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
                    .fetch_one(&pool)
                    .await?;
            if objects == 0 {
                return Err(InfrastructureError::Integrity(
                    "backup contains no schema objects".to_owned(),
                ));
            }
            Ok(())
        }
        .await;
        pool.close().await;
        result
    }
}

impl StoreDatabaseManager {
    /// Verifies a backup file and, when present, its sibling manifest.
    ///
    /// The checksum and byte count in the manifest must match the backup.
    /// Returns the manifest that was checked, if one exists.
    pub async fn verify_backup_with_manifest(
        backup: impl AsRef<Path>,
    ) -> Result<Option<BackupManifest>> {
        let backup = backup.as_ref();
        Self::verify_backup(backup).await?;
        let manifest = Self::load_backup_manifest(backup).await?;
        if let Some(manifest) = manifest.as_ref() {
            Self::verify_manifest(backup, manifest).await?;
        }
        Ok(manifest)
    }
}

impl StoreDatabaseManager {
    async fn verify_manifest(path: &Path, manifest: &BackupManifest) -> Result<()> {
        let (bytes, sha256) = Self::sha256_file(path).await?;
        if bytes != manifest.bytes {
            return Err(InfrastructureError::Integrity(format!(
                "backup size {bytes} does not match manifest size {}",
                manifest.bytes
            )));
        }
        if !sha256.eq_ignore_ascii_case(&manifest.sha256) {
            return Err(InfrastructureError::Integrity(
                "backup checksum does not match its manifest".to_owned(),
            ));
        }
        Ok(())
    }
}

impl StoreDatabaseManager {
    async fn sha256_file(path: &Path) -> Result<(u64, String)> {
        let mut file = tokio::fs::File::open(path).await?;
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = tokio::io::AsyncReadExt::read(&mut file, &mut buffer).await?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            bytes += u64::try_from(read).unwrap_or(u64::MAX);
        }
        Ok((bytes, hex::encode(hasher.finalize())))
    }
}

impl StoreDatabaseManager {
    /// Restores a verified backup into a store database file.
    ///
    /// When a sibling manifest exists, the backup must belong to `store_id`
    /// and match the manifest checksum. The store pool is closed first, and
    /// destination `-wal`/`-shm` files are removed so a stale write-ahead log
    /// cannot be replayed over the restored database. The restored database is
    /// left closed and reopens lazily on the next access.
    pub async fn restore(&self, store_id: &StoreId, backup: impl AsRef<Path>) -> Result<()> {
        let backup = backup.as_ref();
        let destination = self.database_path(store_id)?;
        let canonical_backup = tokio::fs::canonicalize(backup).await?;
        let canonical_destination = if destination.exists() {
            tokio::fs::canonicalize(&destination).await?
        } else {
            destination.clone()
        };
        if canonical_backup == canonical_destination {
            return Err(InfrastructureError::InvalidInput(
                "backup and restore paths must differ".to_owned(),
            ));
        }
        let manifest = Self::load_backup_manifest(backup).await?;
        if let Some(manifest) = manifest.as_ref()
            && manifest.store_id != store_id.as_str()
        {
            return Err(InfrastructureError::InvalidInput(format!(
                "backup belongs to store {} but restore targets {store_id}",
                manifest.store_id
            )));
        }
        Self::verify_backup(&canonical_backup).await?;
        if let Some(manifest) = manifest.as_ref() {
            Self::verify_manifest(&canonical_backup, manifest).await?;
        }
        self.close(store_id).await;
        Self::remove_sidecars(&destination).await?;
        if destination.exists() {
            let previous = destination.with_extension("sqlite.pre-restore");
            tokio::fs::copy(&destination, &previous).await?;
        }
        tokio::fs::copy(&canonical_backup, &destination).await?;
        Self::remove_sidecars(&destination).await?;
        self.migrated.write().await.remove(store_id.as_str());
        Ok(())
    }
}

impl StoreDatabaseManager {
    async fn remove_sidecars(path: &Path) -> Result<()> {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_owned();
            sidecar.push(suffix);
            match tokio::fs::remove_file(PathBuf::from(sidecar)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
