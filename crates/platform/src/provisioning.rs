//! Idempotent store provisioning and lifecycle management.

use std::path::PathBuf;
use std::sync::Arc;

use fastcart_infrastructure::StoreDatabaseManager;
use fastcart_kernel::{StoreId, UserId, now_timestamp};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Row};

use crate::error::{PlatformError, PlatformResult};
use crate::repository::PlatformStore;
use crate::store::{Store, StoreStatus};

/// Outcome of a provisioning run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvisioningReport {
    /// The provisioned store.
    pub store: Store,
    /// Steps executed in this run.
    pub steps: Vec<String>,
    /// Whether the store was already active.
    pub already_active: bool,
}

/// Persistent provisioning job state.
#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct ProvisioningJob {
    /// Job identifier.
    pub id: String,
    /// Store identifier.
    pub store_id: String,
    /// Job status.
    pub status: String,
    /// Current step.
    pub current_step: Option<String>,
    /// Completed steps JSON.
    pub steps: String,
    /// Last error.
    pub last_error: Option<String>,
    /// Attempt counter.
    pub attempts: i64,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

/// Options for completing store deletion.
#[derive(Clone, Debug, Default)]
pub struct DeleteStoreOptions {
    /// When set, a consistent backup is written into this directory first.
    pub backup_dir: Option<PathBuf>,
    /// When true, the store database file is removed after backup.
    pub purge_data: bool,
}

/// Store provisioning and lifecycle service.
#[derive(Clone, Debug)]
pub struct StoreProvisioningService {
    platform: PlatformStore,
    databases: Arc<StoreDatabaseManager>,
}

impl StoreProvisioningService {
    /// Creates the service.
    #[must_use]
    pub fn new(platform: PlatformStore, databases: Arc<StoreDatabaseManager>) -> Self {
        Self {
            platform,
            databases,
        }
    }

    /// Returns the platform store.
    #[must_use]
    pub fn platform(&self) -> &PlatformStore {
        &self.platform
    }

    /// Returns the database manager.
    #[must_use]
    pub fn databases(&self) -> &Arc<StoreDatabaseManager> {
        &self.databases
    }

    /// Provisions a store idempotently.
    ///
    /// Every step is safe to repeat: migrations are versioned, settings use
    /// insert-or-ignore, memberships are upserted, and the store is only marked
    /// active after all data-plane work succeeds.
    pub async fn provision(
        &self,
        store_id: &StoreId,
        owner_id: Option<&UserId>,
    ) -> PlatformResult<ProvisioningReport> {
        let store = self
            .platform
            .get_store(store_id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(store_id.to_string()))?;
        if store.status == StoreStatus::Active {
            return Ok(ProvisioningReport {
                store,
                steps: Vec::new(),
                already_active: true,
            });
        }
        if store.status != StoreStatus::Provisioning {
            return Err(PlatformError::Conflict(format!(
                "store {store_id} is {} and cannot be provisioned",
                store.status
            )));
        }

        let job_id = self.start_job(store_id).await?;
        let mut steps = Vec::new();
        let result = self
            .run_provisioning(&store, owner_id, &job_id, &mut steps)
            .await;
        match result {
            Ok(()) => {
                self.finish_job(&job_id, "succeeded", &steps, None).await?;
                let store = self
                    .platform
                    .get_store(store_id)
                    .await?
                    .ok_or_else(|| PlatformError::StoreNotFound(store_id.to_string()))?;
                Ok(ProvisioningReport {
                    store,
                    steps,
                    already_active: false,
                })
            }
            Err(error) => {
                self.finish_job(&job_id, "failed", &steps, Some(&error.to_string()))
                    .await?;
                Err(error)
            }
        }
    }

    async fn run_provisioning(
        &self,
        store: &Store,
        owner_id: Option<&UserId>,
        job_id: &str,
        steps: &mut Vec<String>,
    ) -> PlatformResult<()> {
        self.update_job_step(job_id, "migrate").await?;
        let database = self.databases.database(&store.id).await?;
        steps.push("migrate".to_owned());

        self.update_job_step(job_id, "seed_settings").await?;
        let now = now_timestamp();
        let settings = [
            ("store.name", store.name.as_str()),
            ("store.locale", store.locale.as_str()),
            ("store.currency_code", store.currency_code.as_str()),
            ("store.timezone", store.timezone.as_str()),
        ];
        for (key, value) in settings {
            sqlx::query("INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES (?, ?, ?)")
                .bind(key)
                .bind(value)
                .bind(now)
                .execute(database.pool())
                .await?;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO sales_channels
             (id, name, description, is_disabled, metadata, created_at, updated_at)
             VALUES ('sc_default', 'Default', 'Default sales channel', 0, '{}', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO inventory_locations
             (id, name, address, metadata, created_at, updated_at)
             VALUES ('loc_default', 'Default location', '{}', '{}', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO tax_categories
             (id, name, code, description, created_at, updated_at)
             VALUES ('tax_default', 'Default', 'default', 'Default tax category', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(database.pool())
        .await?;
        steps.push("seed_settings".to_owned());

        if let Some(owner_id) = owner_id {
            self.update_job_step(job_id, "owner_membership").await?;
            self.platform
                .upsert_membership(owner_id, &store.id, &["owner"])
                .await?;
            steps.push("owner_membership".to_owned());
        }

        self.update_job_step(job_id, "activate").await?;
        self.platform
            .update_status(&store.id, StoreStatus::Active)
            .await?;
        steps.push("activate".to_owned());
        Ok(())
    }

    /// Suspends a store. Storefront traffic stops immediately.
    pub async fn suspend(&self, store_id: &StoreId) -> PlatformResult<Store> {
        self.platform
            .update_status(store_id, StoreStatus::Suspended)
            .await
    }

    /// Resumes a suspended store.
    pub async fn resume(&self, store_id: &StoreId) -> PlatformResult<Store> {
        self.platform
            .update_status(store_id, StoreStatus::Active)
            .await
    }

    /// Marks a store as deleting. The data is preserved until deletion completes.
    pub async fn start_deletion(&self, store_id: &StoreId) -> PlatformResult<Store> {
        self.platform
            .update_status(store_id, StoreStatus::Deleting)
            .await
    }

    /// Completes deletion after an optional backup.
    pub async fn complete_deletion(
        &self,
        store_id: &StoreId,
        options: DeleteStoreOptions,
    ) -> PlatformResult<Store> {
        let current = self
            .platform
            .get_store(store_id)
            .await?
            .ok_or_else(|| PlatformError::StoreNotFound(store_id.to_string()))?;
        if current.status != StoreStatus::Deleting {
            return Err(PlatformError::Conflict(format!(
                "store {store_id} must be in deleting state"
            )));
        }
        if let Some(directory) = options.backup_dir.as_ref() {
            tokio::fs::create_dir_all(directory).await?;
            let path = directory.join(format!("{store_id}-{}.sqlite", now_timestamp()));
            self.databases.backup(store_id, &path).await?;
        }
        self.databases.close(store_id).await;
        let store = self
            .platform
            .update_status(store_id, StoreStatus::Deleted)
            .await?;
        if options.purge_data {
            let path = self.databases.database_path(store_id)?;
            for candidate in [
                path.clone(),
                PathBuf::from(format!("{}-wal", path.display())),
                PathBuf::from(format!("{}-shm", path.display())),
            ] {
                if tokio::fs::try_exists(&candidate).await.unwrap_or(false) {
                    tokio::fs::remove_file(candidate).await?;
                }
            }
        }
        Ok(store)
    }

    /// Returns the latest provisioning job for a store.
    pub async fn latest_job(&self, store_id: &StoreId) -> PlatformResult<Option<ProvisioningJob>> {
        Ok(sqlx::query_as::<_, ProvisioningJob>(
            "SELECT * FROM store_provisioning_jobs WHERE store_id = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(store_id.as_str())
        .fetch_optional(self.platform.pool())
        .await?)
    }

    async fn start_job(&self, store_id: &StoreId) -> PlatformResult<String> {
        let id = format!("prov_{}", uuid::Uuid::now_v7().simple());
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO store_provisioning_jobs
             (id, store_id, status, current_step, steps, attempts, created_at, updated_at)
             VALUES (?, ?, 'running', 'starting', '[]', 1, ?, ?)",
        )
        .bind(&id)
        .bind(store_id.as_str())
        .bind(now)
        .bind(now)
        .execute(self.platform.pool())
        .await?;
        Ok(id)
    }

    async fn update_job_step(&self, job_id: &str, step: &str) -> PlatformResult<()> {
        sqlx::query(
            "UPDATE store_provisioning_jobs SET current_step = ?, updated_at = ? WHERE id = ?",
        )
        .bind(step)
        .bind(now_timestamp())
        .bind(job_id)
        .execute(self.platform.pool())
        .await?;
        Ok(())
    }

    async fn finish_job(
        &self,
        job_id: &str,
        status: &str,
        steps: &[String],
        error: Option<&str>,
    ) -> PlatformResult<()> {
        sqlx::query(
            "UPDATE store_provisioning_jobs SET status = ?, current_step = NULL,
             steps = ?, last_error = ?, updated_at = ? WHERE id = ?",
        )
        .bind(status)
        .bind(serde_json::to_string(steps).unwrap_or_else(|_| "[]".to_owned()))
        .bind(error)
        .bind(now_timestamp())
        .bind(job_id)
        .execute(self.platform.pool())
        .await?;
        Ok(())
    }
}

/// Placeholder for a future resumable provisioning scheduler.
impl StoreProvisioningService {
    /// Re-runs provisioning for stores stuck in the provisioning state.
    pub async fn retry_pending(&self) -> PlatformResult<Vec<ProvisioningReport>> {
        let rows = sqlx::query(
            "SELECT id, store_id FROM store_provisioning_jobs WHERE status = 'running'",
        )
        .fetch_all(self.platform.pool())
        .await?;
        let mut reports = Vec::new();
        for row in rows {
            let store_id = StoreId::from_string(row.try_get::<String, _>("store_id")?);
            match self.provision(&store_id, None).await {
                Ok(report) => reports.push(report),
                Err(error) => {
                    tracing::warn!(store_id = %store_id, error = %error, "provisioning retry failed");
                }
            }
        }
        Ok(reports)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fastcart_infrastructure::{DatabaseOptions, StoreDatabaseManager};
    use fastcart_kernel::StoreId;

    use super::{DeleteStoreOptions, StoreProvisioningService};
    use crate::repository::PlatformStore;
    use crate::store::CreateStoreInput;
    use crate::user::CreateUserInput;

    async fn fixture() -> (
        StoreProvisioningService,
        PlatformStore,
        StoreId,
        fastcart_kernel::UserId,
        tempfile::TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let platform = PlatformStore::in_memory().await.unwrap();
        let databases = Arc::new(StoreDatabaseManager::new(
            dir.path().join("stores"),
            DatabaseOptions::default().with_max_connections(2),
        ));
        let service = StoreProvisioningService::new(platform.clone(), databases);
        let user = platform
            .create_user_with_credential(CreateUserInput::new("owner@example.com"), "hash")
            .await
            .unwrap();
        let store = platform
            .create_store(CreateStoreInput::new("Demo", "demo", "demo.example.com"))
            .await
            .unwrap();
        (service, platform, store.id, user.id, dir)
    }

    #[tokio::test]
    async fn provisioning_creates_database_settings_and_owner() {
        let (service, platform, store_id, owner, _dir) = fixture().await;
        let report = service.provision(&store_id, Some(&owner)).await.unwrap();

        assert_eq!(report.store.status, crate::store::StoreStatus::Active);
        assert!(report.steps.contains(&"migrate".to_owned()));
        assert!(report.steps.contains(&"owner_membership".to_owned()));

        let membership = platform
            .get_membership(&owner, &store_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(membership.roles, vec!["owner"]);

        let database = service.databases().database(&store_id).await.unwrap();
        let name: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'store.name'")
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(name, "Demo");
        let channels: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sales_channels")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(channels, 1);
    }

    #[tokio::test]
    async fn provisioning_is_idempotent() {
        let (service, _platform, store_id, owner, _dir) = fixture().await;
        service.provision(&store_id, Some(&owner)).await.unwrap();
        let second = service.provision(&store_id, Some(&owner)).await.unwrap();
        assert!(second.already_active);
        assert!(second.steps.is_empty());
    }

    #[tokio::test]
    async fn lifecycle_suspend_resume_and_delete() {
        let (service, _platform, store_id, owner, dir) = fixture().await;
        service.provision(&store_id, Some(&owner)).await.unwrap();

        let suspended = service.suspend(&store_id).await.unwrap();
        assert_eq!(suspended.status, crate::store::StoreStatus::Suspended);
        assert!(!suspended.status.can_serve_traffic());

        let resumed = service.resume(&store_id).await.unwrap();
        assert_eq!(resumed.status, crate::store::StoreStatus::Active);

        service.start_deletion(&store_id).await.unwrap();
        let deleted = service
            .complete_deletion(
                &store_id,
                DeleteStoreOptions {
                    backup_dir: Some(dir.path().join("backups")),
                    purge_data: true,
                },
            )
            .await
            .unwrap();
        assert_eq!(deleted.status, crate::store::StoreStatus::Deleted);
        assert!(
            !service
                .databases()
                .database_path(&store_id)
                .unwrap()
                .exists()
        );
        assert!(dir.path().join("backups").exists());
    }

    #[tokio::test]
    async fn cannot_provision_a_suspended_store() {
        let (service, _platform, store_id, owner, _dir) = fixture().await;
        service.provision(&store_id, Some(&owner)).await.unwrap();
        service.suspend(&store_id).await.unwrap();
        assert!(service.provision(&store_id, Some(&owner)).await.is_err());
    }

    #[tokio::test]
    async fn job_history_is_recorded() {
        let (service, _platform, store_id, owner, _dir) = fixture().await;
        service.provision(&store_id, Some(&owner)).await.unwrap();
        let job = service.latest_job(&store_id).await.unwrap().unwrap();
        assert_eq!(job.status, "succeeded");
        assert!(job.steps.contains("activate"));
    }
}
