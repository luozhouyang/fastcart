//! Store-scoped bindings from a capability id to one plugin provider.

use fastcart_infrastructure::{AuditActor, StoreDatabase};
use fastcart_kernel::now_timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use crate::contribution::Contribution;
use crate::error::PluginError;
use crate::manifest::PluginManifest;

/// A stored capability binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CapabilityBinding {
    /// Capability identifier.
    pub capability_id: String,
    /// Bound contribution version.
    pub version: u32,
    /// Plugin that provides the capability.
    pub plugin_id: String,
    /// Store configuration passed to the theme.
    #[schema(value_type = Object)]
    pub config: Value,
    /// Whether the storefront route resolves this binding.
    pub enabled: bool,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

type BindingRow = (String, i64, String, String, i64, i64, i64);

fn into_binding(row: BindingRow) -> Result<CapabilityBinding, PluginError> {
    let (capability_id, version, plugin_id, config_json, enabled, created_at, updated_at) = row;
    let config: Value = serde_json::from_str(&config_json)
        .map_err(|error| PluginError::Database(error.to_string()))?;
    let version = u32::try_from(version)
        .map_err(|_| PluginError::Database("invalid capability version".to_owned()))?;
    Ok(CapabilityBinding {
        capability_id,
        version,
        plugin_id,
        config,
        enabled: enabled != 0,
        created_at,
        updated_at,
    })
}

/// Capability binding operations for one store.
#[derive(Clone, Debug)]
pub struct CapabilityBindingService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl CapabilityBindingService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records binding mutations in the store audit outbox.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Lists bindings ordered by capability id.
    ///
    /// # Errors
    ///
    /// Returns database errors.
    pub async fn list(&self) -> Result<Vec<CapabilityBinding>, PluginError> {
        let rows: Vec<BindingRow> = sqlx::query_as(
            "SELECT capability_id, version, plugin_id, config_json, enabled, created_at, updated_at \
             FROM store_capability_bindings ORDER BY capability_id ASC",
        )
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter().map(into_binding).collect()
    }

    /// Binds one capability to the provider declared in `plugin_id`.
    ///
    /// The plugin must be installed and its current manifest must declare the
    /// exact capability id and version. The config must be an object and must
    /// satisfy the contribution's JSON Schema when one is declared.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::NotFound`] when the plugin is not installed,
    /// [`PluginError::CapabilityNotFound`] when the manifest does not provide
    /// the pair, and [`PluginError::InvalidSettings`] for a bad config.
    #[allow(clippy::too_many_arguments)]
    pub async fn bind(
        &self,
        plugin_id: &str,
        capability_id: &str,
        version: u32,
        config: Value,
        enabled: bool,
    ) -> Result<CapabilityBinding, PluginError> {
        self.ensure_capability(plugin_id, capability_id, version, &config)
            .await?;
        let now = now_timestamp();
        let config_json = serde_json::to_string(&config)
            .map_err(|error| PluginError::Database(error.to_string()))?;
        let mut tx = self.db.begin().await?;
        sqlx::query(
            "INSERT INTO store_capability_bindings \
             (capability_id, version, plugin_id, config_json, enabled, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(capability_id) DO UPDATE SET version = excluded.version, \
             plugin_id = excluded.plugin_id, config_json = excluded.config_json, \
             enabled = excluded.enabled, updated_at = excluded.updated_at",
        )
        .bind(capability_id)
        .bind(i64::from(version))
        .bind(plugin_id)
        .bind(config_json)
        .bind(i64::from(enabled))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        self.append_audit(
            &mut tx,
            "capability.bound",
            capability_id,
            serde_json::json!({
                "plugin_id": plugin_id,
                "version": version,
                "enabled": enabled,
            }),
        )
        .await?;
        tx.commit().await?;
        self.get(capability_id)
            .await?
            .ok_or_else(|| PluginError::Database("binding disappeared after write".to_owned()))
    }

    /// Removes one binding if it exists.
    ///
    /// # Errors
    ///
    /// Returns database errors.
    pub async fn unbind(&self, capability_id: &str) -> Result<bool, PluginError> {
        let mut tx = self.db.begin().await?;
        let result = sqlx::query("DELETE FROM store_capability_bindings WHERE capability_id = ?")
            .bind(capability_id)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() > 0 {
            self.append_audit(
                &mut tx,
                "capability.unbound",
                capability_id,
                serde_json::json!({}),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(result.rows_affected() > 0)
    }

    /// Returns one binding.
    ///
    /// # Errors
    ///
    /// Returns database errors.
    pub async fn get(&self, capability_id: &str) -> Result<Option<CapabilityBinding>, PluginError> {
        let row: Option<BindingRow> = sqlx::query_as(
            "SELECT capability_id, version, plugin_id, config_json, enabled, created_at, updated_at \
             FROM store_capability_bindings WHERE capability_id = ?",
        )
        .bind(capability_id)
        .fetch_optional(self.db.pool())
        .await?;
        row.map(into_binding).transpose()
    }

    async fn ensure_capability(
        &self,
        plugin_id: &str,
        capability_id: &str,
        version: u32,
        config: &Value,
    ) -> Result<(), PluginError> {
        if !valid_capability_id(capability_id) || version == 0 {
            return Err(PluginError::CapabilityNotFound(format!(
                "{capability_id}@{version}"
            )));
        }
        let row: Option<(String,)> =
            sqlx::query_as("SELECT manifest_json FROM plugin_installations WHERE plugin_id = ?")
                .bind(plugin_id)
                .fetch_optional(self.db.pool())
                .await?;
        let Some((manifest_json,)) = row else {
            return Err(PluginError::NotFound(plugin_id.to_owned()));
        };
        let manifest = PluginManifest::parse(manifest_json.as_bytes())?;
        let contribution = manifest
            .provides
            .iter()
            .find_map(|contribution| match contribution {
                Contribution::StorefrontCapability(capability)
                    if capability.id == capability_id && capability.version == version =>
                {
                    Some(capability)
                }
                _ => None,
            });
        let Some(contribution) = contribution else {
            return Err(PluginError::CapabilityNotFound(format!(
                "{capability_id}@{version}"
            )));
        };
        if !config.is_object() {
            return Err(PluginError::InvalidSettings(
                "capability config must be a JSON object".to_owned(),
            ));
        }
        if let Some(schema) = contribution.config_schema.as_ref() {
            let validator = jsonschema::validator_for(schema)
                .map_err(|error| PluginError::InvalidSettings(error.to_string()))?;
            if !validator.is_valid(config) {
                let message = validator
                    .iter_errors(config)
                    .next()
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "config does not match the contribution schema".to_owned());
                return Err(PluginError::InvalidSettings(message));
            }
        }
        Ok(())
    }

    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        action: &str,
        capability_id: &str,
        data: Value,
    ) -> Result<(), PluginError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, "capability", capability_id, data)
                .await
                .map_err(|error| PluginError::Database(error.to_string()))?;
        }
        Ok(())
    }
}

fn valid_capability_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && !value.starts_with('-')
        && !value.ends_with('-')
}
