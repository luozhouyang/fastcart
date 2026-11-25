//! SQLite-backed theme registry and activation service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, PageResult, Pagination, ThemeActivationId, ThemeId, Timestamp, now_timestamp,
};
use fastcart_storefront::{StorefrontError, ThemeManifest, ThemeRequirements};
use serde_json::Value;
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    RegisterThemeInput, Theme, ThemeActivationRecord, ThemeSettingsRevision,
    ThemeSettingsRevisionStatus, UpdateThemeSettingsInput,
};

const THEME_COLUMNS: &str = "id, manifest_id, name, version, settings_schema, settings, \
     requires_json, draft_settings, settings_revision, is_active, created_at, updated_at";

const SETTINGS_REVISION_COLUMNS: &str = "id, theme_id, revision, settings, status, \
     created_by, created_at";

/// Application service for the storefront theme registry.
#[derive(Clone, Debug)]
pub struct ThemeService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

mod activation;
mod registry;
mod settings;

impl ThemeService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }
}

impl ThemeService {
    /// Records theme mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }
}

impl ThemeService {
    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }
}

impl ThemeService {
    fn audit_actor_id(&self) -> Option<String> {
        self.audit.as_ref().map(|audit| audit.actor_id().to_owned())
    }
}

impl ThemeService {
    /// Writes or replaces one settings revision row.
    #[allow(clippy::too_many_arguments)]
    /// Deletes working-copy revisions for a theme.
    ///
    /// Used when the live settings are changed directly or the working draft is
    /// discarded, so revision history never shows a draft that is not current.
    async fn discard_settings_drafts(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        theme_id: &ThemeId,
    ) -> Result<(), AppError> {
        sqlx::query("DELETE FROM theme_settings_revisions WHERE theme_id = ? AND status = 'draft'")
            .bind(theme_id.as_str())
            .execute(&mut **tx)
            .await
            .map_err(database_error)?;
        Ok(())
    }
}

impl ThemeService {
    // One row write needs every column and the audit actor; grouping them in a
    // private struct would not make the call sites clearer.
    #[allow(clippy::too_many_arguments)]
    async fn write_settings_revision(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        theme_id: &ThemeId,
        revision: i64,
        settings_json: &str,
        status: ThemeSettingsRevisionStatus,
        created_by: Option<&str>,
        created_at: Timestamp,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO theme_settings_revisions \
             (id, theme_id, revision, settings, status, created_by, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (theme_id, revision) DO UPDATE SET \
             settings = excluded.settings, status = excluded.status, \
             created_by = excluded.created_by, created_at = excluded.created_at",
        )
        .bind(format!("{}:{revision}", theme_id.as_str()))
        .bind(theme_id.as_str())
        .bind(revision)
        .bind(settings_json)
        .bind(status.as_str())
        .bind(created_by)
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

impl ThemeService {
    async fn manifest_version_exists(
        &self,
        manifest_id: &str,
        version: &str,
    ) -> Result<bool, AppError> {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM themes WHERE manifest_id = ? AND version = ?")
                .bind(manifest_id)
                .bind(version)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        Ok(count > 0)
    }
}

impl ThemeService {
    async fn fetch_row(&self, id: &ThemeId) -> Result<ThemeRow, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?");
        sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("theme", id))
    }
}

impl ThemeService {
    async fn fetch_row_in(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        id: &ThemeId,
    ) -> Result<ThemeRow, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?");
        sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("theme", id))
    }
}

fn storefront_error(error: StorefrontError) -> AppError {
    AppError::validation(error.to_string())
}

fn unique_conflict(error: sqlx::Error, message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(message)
        }
        _ => database_error(error),
    }
}

fn validate_settings_schema(schema: &Value) -> Result<(), AppError> {
    if !schema.is_object() {
        return Err(AppError::validation(
            "settings_schema must be a JSON object",
        ));
    }
    jsonschema::validator_for(schema)
        .map_err(|error| AppError::validation(format!("invalid settings schema: {error}")))?;
    Ok(())
}

/// Rejects a settings write when the caller's revision is stale.
fn ensure_settings_revision(theme: &Theme, expected: Option<i64>) -> Result<(), AppError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    if theme.settings_revision != expected {
        return Err(AppError::conflict_details(
            format!(
                "theme settings revision {} does not match the expected revision {expected}",
                theme.settings_revision
            ),
            serde_json::json!({ "current_revision": theme.settings_revision }),
        ));
    }
    Ok(())
}

fn validate_settings(schema: Option<&Value>, settings: &Value) -> Result<(), AppError> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| AppError::validation(format!("invalid settings schema: {error}")))?;
    validator
        .validate(settings)
        .map_err(|error| AppError::validation(format!("invalid theme settings: {error}")))
}

fn serialize_json<T: serde::Serialize>(value: &T, field: &str) -> Result<String, AppError> {
    serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("could not serialize {field}: {error}")))
}

#[derive(Debug, FromRow)]
struct ThemeRow {
    id: String,
    manifest_id: String,
    name: String,
    version: String,
    settings_schema: Option<String>,
    settings: String,
    requires_json: Option<String>,
    draft_settings: Option<String>,
    settings_revision: i64,
    is_active: i64,
    created_at: i64,
    updated_at: i64,
}

impl ThemeRow {
    fn into_theme(self) -> Result<Theme, AppError> {
        let requires: Option<ThemeRequirements> = match self.requires_json {
            Some(raw) => Some(serde_json::from_str(&raw).map_err(|error| {
                AppError::internal(format!("invalid stored theme requirements: {error}"))
            })?),
            None => None,
        };
        let manifest = ThemeManifest {
            id: self.manifest_id,
            name: self.name,
            version: self.version,
            requires,
        };
        let settings_schema: Option<Value> = match self.settings_schema {
            Some(raw) => Some(serde_json::from_str(&raw).map_err(|error| {
                AppError::internal(format!("invalid stored theme settings schema: {error}"))
            })?),
            None => None,
        };
        let settings: Value = serde_json::from_str(&self.settings).map_err(|error| {
            AppError::internal(format!("invalid stored theme settings: {error}"))
        })?;
        let draft_settings: Option<Value> = match self.draft_settings {
            Some(raw) => Some(serde_json::from_str(&raw).map_err(|error| {
                AppError::internal(format!("invalid stored theme settings draft: {error}"))
            })?),
            None => None,
        };

        Ok(Theme {
            id: ThemeId::from_string(self.id),
            manifest,
            settings_schema,
            settings,
            draft_settings,
            settings_revision: self.settings_revision,
            is_active: self.is_active != 0,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct ThemeSettingsRevisionRow {
    id: String,
    theme_id: String,
    revision: i64,
    settings: String,
    status: String,
    created_by: Option<String>,
    created_at: i64,
}

impl ThemeSettingsRevisionRow {
    fn into_revision(self) -> Result<ThemeSettingsRevision, AppError> {
        let settings: Value = serde_json::from_str(&self.settings).map_err(|error| {
            AppError::internal(format!("invalid stored theme settings revision: {error}"))
        })?;
        let status = ThemeSettingsRevisionStatus::parse(&self.status).ok_or_else(|| {
            AppError::internal(format!(
                "invalid stored theme settings revision status: {}",
                self.status
            ))
        })?;
        Ok(ThemeSettingsRevision {
            id: self.id,
            theme_id: ThemeId::from_string(self.theme_id),
            revision: self.revision,
            settings,
            status,
            created_by: self.created_by,
            created_at: self.created_at,
        })
    }
}
