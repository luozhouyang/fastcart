//! Store-scoped plugin installation service.

use std::collections::HashSet;
use std::sync::Arc;

use fastcart_infrastructure::{AuditActor, StoreDatabase};
use fastcart_kernel::now_timestamp;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::artifact::{ArtifactStore, ArtifactSummary};
use crate::bundle::RuntimeBundle;
use crate::error::PluginError;
use crate::manifest::{PluginManifest, validate_settings};
use crate::model::InstalledPlugin;

const COLUMNS: &str = "id, plugin_id, digest, version, manifest_json, enabled, settings_json, \
     settings_revision, granted_scopes, previous_digest, installed_by, installed_at, updated_at";

#[derive(Debug, FromRow)]
struct InstallationRow {
    id: String,
    plugin_id: String,
    digest: String,
    version: String,
    manifest_json: String,
    enabled: i64,
    settings_json: String,
    settings_revision: i64,
    granted_scopes: String,
    previous_digest: Option<String>,
    installed_by: Option<String>,
    installed_at: i64,
    updated_at: i64,
}

impl InstallationRow {
    fn into_plugin(self) -> Result<InstalledPlugin, PluginError> {
        let manifest = PluginManifest::parse(self.manifest_json.as_bytes())?;
        let settings: Value = serde_json::from_str(&self.settings_json)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let granted_scopes: Vec<String> = serde_json::from_str(&self.granted_scopes)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let _ = self.id;
        let _ = self.installed_by;
        Ok(InstalledPlugin {
            id: self.plugin_id,
            name: manifest.name,
            version: self.version,
            digest: self.digest,
            previous_digest: self.previous_digest,
            enabled: self.enabled != 0,
            settings,
            settings_revision: self.settings_revision,
            granted_scopes,
            installed_at: self.installed_at,
            updated_at: self.updated_at,
        })
    }
}

/// Plugin installation operations for one store.
#[derive(Clone, Debug)]
pub struct PluginService {
    db: StoreDatabase,
    artifacts: Arc<ArtifactStore>,
    audit: Option<AuditActor>,
}

impl PluginService {
    /// Creates a service bound to one store database and artifact store.
    #[must_use]
    pub fn new(db: StoreDatabase, artifacts: Arc<ArtifactStore>) -> Self {
        Self {
            db,
            artifacts,
            audit: None,
        }
    }

    /// Records plugin mutations in the store audit outbox.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        plugin_id: &str,
        data: Value,
    ) -> Result<(), PluginError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, "plugin", plugin_id, data)
                .await
                .map_err(|error| PluginError::Database(error.to_string()))?;
        }
        Ok(())
    }

    /// Lists installed plugins ordered by id.
    ///
    /// # Errors
    ///
    /// Returns database errors.
    pub async fn list(&self) -> Result<Vec<InstalledPlugin>, PluginError> {
        let sql = format!("SELECT {COLUMNS} FROM plugin_installations ORDER BY plugin_id ASC");
        let rows = sqlx::query_as::<_, InstallationRow>(&sql)
            .fetch_all(self.db.pool())
            .await?;
        rows.into_iter().map(InstallationRow::into_plugin).collect()
    }

    /// Returns one installed plugin.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] when absent.
    pub async fn get(&self, plugin_id: &str) -> Result<InstalledPlugin, PluginError> {
        self.row(plugin_id).await?.into_plugin()
    }

    /// Returns the installation and its manifest.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] when absent.
    pub async fn detail(
        &self,
        plugin_id: &str,
    ) -> Result<(InstalledPlugin, PluginManifest), PluginError> {
        let row = self.row(plugin_id).await?;
        let manifest = PluginManifest::parse(row.manifest_json.as_bytes())?;
        let plugin = row.into_plugin()?;
        Ok((plugin, manifest))
    }

    async fn row(&self, plugin_id: &str) -> Result<InstallationRow, PluginError> {
        let sql = format!("SELECT {COLUMNS} FROM plugin_installations WHERE plugin_id = ?");
        sqlx::query_as::<_, InstallationRow>(&sql)
            .bind(plugin_id)
            .fetch_optional(self.db.pool())
            .await?
            .ok_or_else(|| PluginError::NotFound(plugin_id.to_owned()))
    }

    /// Installs an archive as a disabled plugin.
    ///
    /// # Errors
    ///
    /// Returns validation, storage and duplicate errors.
    pub async fn install(&self, bytes: &[u8]) -> Result<InstalledPlugin, PluginError> {
        let inspected = self.artifacts.inspect(bytes)?;
        let plugin_id = inspected.manifest.id.clone();
        if self.row_optional(&plugin_id).await?.is_some() {
            return Err(PluginError::Duplicate(plugin_id));
        }
        let stored = self.artifacts.install(bytes)?;
        let scopes = scope_names(&stored.manifest)?;
        let scopes_json = serde_json::to_string(&scopes)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let manifest_json = serde_json::to_string(&stored.manifest)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let settings = json!({});
        let now = now_timestamp();
        let row_id = format!("pli_{}", uuid::Uuid::now_v7().simple());

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO plugin_installations \
             (id, plugin_id, digest, version, manifest_json, enabled, settings_json, \
              settings_revision, granted_scopes, previous_digest, installed_by, installed_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, 0, ?, 1, ?, NULL, ?, ?, ?)",
        )
        .bind(&row_id)
        .bind(&plugin_id)
        .bind(&stored.digest)
        .bind(&stored.manifest.version)
        .bind(&manifest_json)
        .bind(settings.to_string())
        .bind(&scopes_json)
        .bind(self.audit.as_ref().map(|audit| audit.actor_id().to_owned()))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(map_unique_error)?;
        for event_type in &stored.manifest.capabilities.events {
            sqlx::query(
                "INSERT OR IGNORE INTO plugin_event_subscriptions                  (plugin_id, event_type, enabled) VALUES (?, ?, 1)",
            )
            .bind(&plugin_id)
            .bind(event_type)
            .execute(&mut *tx)
            .await?;
        }
        self.append_audit(
            &mut tx,
            "plugin.installed",
            &plugin_id,
            json!({ "digest": stored.digest, "version": stored.manifest.version }),
        )
        .await?;
        tx.commit().await?;
        self.get(&plugin_id).await
    }

    /// Upgrades an installed plugin to a new artifact.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::ScopesNotAccepted`] when the new version requests
    /// additional capabilities without `accept_new_scopes`.
    pub async fn upgrade(
        &self,
        plugin_id: &str,
        bytes: &[u8],
        accept_new_scopes: bool,
    ) -> Result<InstalledPlugin, PluginError> {
        let current = self.row(plugin_id).await?;
        let inspected = self.artifacts.inspect(bytes)?;
        if inspected.manifest.id != plugin_id {
            return Err(PluginError::InvalidManifest(format!(
                "archive contains plugin {}, expected {plugin_id}",
                inspected.manifest.id
            )));
        }
        if inspected.digest == current.digest {
            return Err(PluginError::Duplicate(plugin_id.to_owned()));
        }
        let current_scopes: Vec<String> = serde_json::from_str(&current.granted_scopes)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let next_scopes = scope_names(&inspected.manifest)?;
        let added: Vec<String> = next_scopes
            .iter()
            .filter(|scope| !current_scopes.contains(scope))
            .cloned()
            .collect();
        if !added.is_empty() && !accept_new_scopes {
            return Err(PluginError::ScopesNotAccepted(added.join(", ")));
        }
        let settings: Value = serde_json::from_str(&current.settings_json)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        validate_settings(&inspected.manifest, &settings)?;
        let stored = self.artifacts.install(bytes)?;
        let manifest_json = serde_json::to_string(&stored.manifest)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let scopes_json = serde_json::to_string(&next_scopes)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let now = now_timestamp();

        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE plugin_installations SET digest = ?, version = ?, manifest_json = ?, \
             granted_scopes = ?, previous_digest = ?, updated_at = ? WHERE plugin_id = ?",
        )
        .bind(&stored.digest)
        .bind(&stored.manifest.version)
        .bind(&manifest_json)
        .bind(&scopes_json)
        .bind(&current.digest)
        .bind(now)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM plugin_event_subscriptions WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;
        for event_type in &stored.manifest.capabilities.events {
            sqlx::query(
                "INSERT OR IGNORE INTO plugin_event_subscriptions                  (plugin_id, event_type, enabled) VALUES (?, ?, 1)",
            )
            .bind(plugin_id)
            .bind(event_type)
            .execute(&mut *tx)
            .await?;
        }
        self.append_audit(
            &mut tx,
            "plugin.upgraded",
            plugin_id,
            json!({
                "from": current.digest,
                "to": stored.digest,
                "version": stored.manifest.version,
            }),
        )
        .await?;
        tx.commit().await?;
        self.get(plugin_id).await
    }

    /// Enables a plugin.
    ///
    /// # Errors
    ///
    /// Fails when the artifact is missing or the settings are invalid.
    pub async fn enable(&self, plugin_id: &str) -> Result<InstalledPlugin, PluginError> {
        let row = self.row(plugin_id).await?;
        if row.enabled != 0 {
            return row.into_plugin();
        }
        let artifact = self.artifacts.load(&row.digest)?;
        let settings: Value = serde_json::from_str(&row.settings_json)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        validate_settings(&artifact.manifest, &settings)?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE plugin_installations SET enabled = 1, updated_at = ? WHERE plugin_id = ?",
        )
        .bind(now)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
        self.append_audit(
            &mut tx,
            "plugin.enabled",
            plugin_id,
            json!({ "digest": row.digest }),
        )
        .await?;
        tx.commit().await?;
        self.get(plugin_id).await
    }

    /// Disables a plugin.
    ///
    /// # Errors
    ///
    /// Returns database errors.
    pub async fn disable(&self, plugin_id: &str) -> Result<InstalledPlugin, PluginError> {
        let row = self.row(plugin_id).await?;
        if row.enabled == 0 {
            return row.into_plugin();
        }
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE plugin_installations SET enabled = 0, updated_at = ? WHERE plugin_id = ?",
        )
        .bind(now)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
        self.append_audit(&mut tx, "plugin.disabled", plugin_id, json!({}))
            .await?;
        tx.commit().await?;
        self.get(plugin_id).await
    }

    /// Replaces the settings object after schema validation.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidSettings`] on a schema violation.
    pub async fn update_settings(
        &self,
        plugin_id: &str,
        settings: Value,
    ) -> Result<InstalledPlugin, PluginError> {
        let row = self.row(plugin_id).await?;
        let manifest = PluginManifest::parse(row.manifest_json.as_bytes())?;
        validate_settings(&manifest, &settings)?;
        let now = now_timestamp();
        let revision = row.settings_revision + 1;
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE plugin_installations SET settings_json = ?, settings_revision = ?, updated_at = ? \
             WHERE plugin_id = ?",
        )
        .bind(settings.to_string())
        .bind(revision)
        .bind(now)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
        self.append_audit(
            &mut tx,
            "plugin.settings_updated",
            plugin_id,
            json!({ "revision": revision }),
        )
        .await?;
        tx.commit().await?;
        self.get(plugin_id).await
    }

    /// Swaps the active and previous digests.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NoPreviousDigest`] when no rollback target exists.
    pub async fn rollback(&self, plugin_id: &str) -> Result<InstalledPlugin, PluginError> {
        let row = self.row(plugin_id).await?;
        let previous = row
            .previous_digest
            .clone()
            .ok_or(PluginError::NoPreviousDigest)?;
        let artifact = self.artifacts.load(&previous)?;
        let settings: Value = serde_json::from_str(&row.settings_json)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        validate_settings(&artifact.manifest, &settings)?;
        let manifest_json = serde_json::to_string(&artifact.manifest)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "UPDATE plugin_installations SET digest = ?, version = ?, manifest_json = ?, \
             previous_digest = ?, updated_at = ? WHERE plugin_id = ?",
        )
        .bind(&artifact.digest)
        .bind(&artifact.manifest.version)
        .bind(&manifest_json)
        .bind(&row.digest)
        .bind(now)
        .bind(plugin_id)
        .execute(&mut *tx)
        .await?;
        self.append_audit(
            &mut tx,
            "plugin.rolled_back",
            plugin_id,
            json!({ "from": row.digest, "to": artifact.digest }),
        )
        .await?;
        tx.commit().await?;
        self.get(plugin_id).await
    }

    /// Removes a disabled plugin installation. Artifacts are kept for GC.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Enabled`] when the plugin is still enabled.
    pub async fn remove(&self, plugin_id: &str) -> Result<(), PluginError> {
        let row = self.row(plugin_id).await?;
        if row.enabled != 0 {
            return Err(PluginError::Enabled(plugin_id.to_owned()));
        }
        let mut tx = self.db.begin().await?;
        sqlx::query("DELETE FROM plugin_event_subscriptions WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM plugin_deliveries WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM plugin_jobs WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id = ?")
            .bind(plugin_id)
            .execute(&mut *tx)
            .await?;
        self.append_audit(
            &mut tx,
            "plugin.removed",
            plugin_id,
            json!({ "digest": row.digest }),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Returns the runtime bundle for an enabled plugin.
    ///
    /// # Errors
    ///
    /// Fails when the plugin is disabled or the digest is not installed.
    pub async fn bundle(
        &self,
        plugin_id: &str,
        digest: &str,
    ) -> Result<RuntimeBundle, PluginError> {
        let row = self.row(plugin_id).await?;
        if row.enabled == 0 {
            return Err(PluginError::Disabled(plugin_id.to_owned()));
        }
        if row.digest != digest && row.previous_digest.as_deref() != Some(digest) {
            return Err(PluginError::NotFound(format!("{plugin_id}@{digest}")));
        }
        self.artifacts.read_bundle(digest)
    }

    /// Records a failed mutation in the audit outbox on a best-effort basis.
    ///
    /// Failures are logged but never replace the original error.
    pub async fn record_failure(&self, action: &str, error: &PluginError) {
        let Some(audit) = &self.audit else {
            return;
        };
        let data = json!({
            "code": error.code(),
            "message": error.to_string(),
        });
        if let Err(audit_error) = audit
            .append(
                self.db.pool(),
                &format!("{action}_failed"),
                "plugin",
                "unknown",
                data,
            )
            .await
        {
            tracing::warn!(error = %audit_error, action, "failed to record plugin audit failure");
        }
    }

    /// Lists artifacts stored on disk.
    ///
    /// # Errors
    ///
    /// Returns storage errors.
    pub fn artifacts(&self) -> Result<Vec<ArtifactSummary>, PluginError> {
        self.artifacts.list()
    }

    async fn row_optional(&self, plugin_id: &str) -> Result<Option<InstallationRow>, PluginError> {
        let sql = format!("SELECT {COLUMNS} FROM plugin_installations WHERE plugin_id = ?");
        Ok(sqlx::query_as::<_, InstallationRow>(&sql)
            .bind(plugin_id)
            .fetch_optional(self.db.pool())
            .await?)
    }
}

/// Returns every digest referenced by a store, including rollback targets.
///
/// # Errors
///
/// Returns database errors.
pub async fn referenced_digests(db: &StoreDatabase) -> Result<HashSet<String>, PluginError> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT digest, previous_digest FROM plugin_installations")
            .fetch_all(db.pool())
            .await?;
    let mut digests = HashSet::new();
    for (digest, previous) in rows {
        digests.insert(digest);
        if let Some(previous) = previous {
            digests.insert(previous);
        }
    }
    Ok(digests)
}

fn scope_names(manifest: &PluginManifest) -> Result<Vec<String>, PluginError> {
    Ok(manifest
        .scopes()?
        .into_iter()
        .map(|scope| scope.as_str().to_owned())
        .collect())
}

fn map_unique_error(error: sqlx::Error) -> PluginError {
    let message = error.to_string();
    if message.contains("UNIQUE") {
        PluginError::Duplicate("plugin".to_owned())
    } else {
        PluginError::Database(message)
    }
}
