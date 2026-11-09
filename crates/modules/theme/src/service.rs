//! SQLite-backed theme registry and activation service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, PageResult, Pagination, ThemeActivationId, ThemeId, Timestamp, now_timestamp,
};
use fastcart_storefront::{StorefrontError, ThemeManifest};
use serde_json::Value;
use sqlx::{FromRow, Sqlite, Transaction};

use crate::domain::{
    RegisterThemeInput, Theme, ThemeActivationRecord, ThemeSettingsRevision,
    ThemeSettingsRevisionStatus, UpdateThemeSettingsInput,
};

const THEME_COLUMNS: &str = "id, manifest_id, name, version, settings_schema, settings, \
     draft_settings, settings_revision, is_active, created_at, updated_at";

const SETTINGS_REVISION_COLUMNS: &str = "id, theme_id, revision, settings, status, \
     created_by, created_at";

/// Application service for the storefront theme registry.
#[derive(Clone, Debug)]
pub struct ThemeService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl ThemeService {
    /// Creates a service bound to one store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records theme mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating method appends its audit entry in
    /// the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

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

    fn audit_actor_id(&self) -> Option<String> {
        self.audit.as_ref().map(|audit| audit.actor_id().to_owned())
    }

    /// Registers theme metadata, its settings schema and initial settings.
    ///
    /// The same manifest may be registered more than once at different
    /// versions; registering an existing `(manifest_id, version)` pair is a
    /// conflict.
    pub async fn register(&self, input: RegisterThemeInput) -> Result<Theme, AppError> {
        let RegisterThemeInput {
            manifest,
            settings_schema,
            settings,
        } = input;

        manifest.validate().map_err(storefront_error)?;
        if let Some(schema) = settings_schema.as_ref() {
            validate_settings_schema(schema)?;
        }
        let settings = settings.unwrap_or_else(|| serde_json::json!({}));
        validate_settings(settings_schema.as_ref(), &settings)?;

        if self
            .manifest_version_exists(&manifest.id, &manifest.version)
            .await?
        {
            return Err(AppError::conflict(format!(
                "theme version already registered: {}@{}",
                manifest.id, manifest.version
            )));
        }

        let id = ThemeId::new();
        let now = now_timestamp();
        let created_by = self.audit_actor_id();
        let settings_schema_json = match settings_schema.as_ref() {
            Some(schema) => Some(serialize_json(schema, "settings_schema")?),
            None => None,
        };
        let settings_json = serialize_json(&settings, "settings")?;

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO themes \
             (id, manifest_id, name, version, settings_schema, settings, is_active, \
              created_at, updated_at, draft_settings, settings_revision) \
             VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, NULL, 1)",
        )
        .bind(id.as_str())
        .bind(&manifest.id)
        .bind(&manifest.name)
        .bind(&manifest.version)
        .bind(settings_schema_json.as_deref())
        .bind(&settings_json)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, "theme version already registered"))?;
        // The initial settings are live, so they form revision 1.
        self.write_settings_revision(
            &mut tx,
            &id,
            1,
            &settings_json,
            ThemeSettingsRevisionStatus::Published,
            created_by.as_deref(),
            now,
        )
        .await?;
        self.append_audit(
            &mut tx,
            "theme.registered",
            "theme",
            id.as_str(),
            serde_json::json!({
                "manifest_id": manifest.id,
                "version": manifest.version,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(&id).await
    }

    /// Lists registered themes, newest first.
    pub async fn list(&self, pagination: Pagination) -> Result<PageResult<Theme>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let sql = format!(
            "SELECT {THEME_COLUMNS} FROM themes ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let data = rows
            .into_iter()
            .map(ThemeRow::into_theme)
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(PageResult::new(data, count, pagination))
    }

    /// Loads a theme by internal identifier.
    pub async fn get(&self, id: &ThemeId) -> Result<Theme, AppError> {
        self.fetch_row(id).await?.into_theme()
    }

    /// Returns the active theme, if one has been activated.
    pub async fn active(&self) -> Result<Option<Theme>, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE is_active = 1 LIMIT 1");
        let row = sqlx::query_as::<_, ThemeRow>(&sql)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        row.map(ThemeRow::into_theme).transpose()
    }

    /// Replaces a theme's settings after schema validation.
    ///
    /// This keeps the historical immediate-publish contract: `settings` becomes
    /// live at once. A matching draft and a published revision snapshot are
    /// written in the same transaction.
    pub async fn update_settings(
        &self,
        id: &ThemeId,
        input: UpdateThemeSettingsInput,
    ) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let theme = self.fetch_row_in(&mut tx, id).await?.into_theme()?;
        validate_settings(theme.settings_schema.as_ref(), &input.settings)?;

        let settings_json = serialize_json(&input.settings, "settings")?;
        let changed = theme.settings != input.settings;
        let now = now_timestamp();
        let created_by = self.audit_actor_id();

        if changed {
            let revision = theme.settings_revision + 1;
            sqlx::query(
                "UPDATE themes SET settings = ?, draft_settings = NULL, settings_revision = ?, \
                 updated_at = ? WHERE id = ?",
            )
            .bind(&settings_json)
            .bind(revision)
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            self.discard_settings_drafts(&mut tx, id).await?;
            self.write_settings_revision(
                &mut tx,
                id,
                revision,
                &settings_json,
                ThemeSettingsRevisionStatus::Published,
                created_by.as_deref(),
                now,
            )
            .await?;
            self.append_audit(
                &mut tx,
                "theme.settings_updated",
                "theme",
                id.as_str(),
                serde_json::json!({"manifest_id": theme.manifest.id, "revision": revision}),
            )
            .await?;
        } else {
            // No live change: still discard any working draft so the state is
            // exactly the published settings.
            sqlx::query("UPDATE themes SET draft_settings = NULL, updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            self.discard_settings_drafts(&mut tx, id).await?;
        }
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Saves a settings draft without changing the live settings.
    ///
    /// The draft revision reuses the next revision number until it is
    /// published, so repeated saves replace one working copy.
    pub async fn save_settings_draft(
        &self,
        id: &ThemeId,
        settings: Value,
        created_by: Option<String>,
        expected_revision: Option<i64>,
    ) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let theme = self.fetch_row_in(&mut tx, id).await?.into_theme()?;
        ensure_settings_revision(&theme, expected_revision)?;
        validate_settings(theme.settings_schema.as_ref(), &settings)?;

        let settings_json = serialize_json(&settings, "settings")?;
        let revision = theme.settings_revision + 1;
        let now = now_timestamp();

        sqlx::query("UPDATE themes SET draft_settings = ?, updated_at = ? WHERE id = ?")
            .bind(&settings_json)
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.write_settings_revision(
            &mut tx,
            id,
            revision,
            &settings_json,
            ThemeSettingsRevisionStatus::Draft,
            created_by.as_deref(),
            now,
        )
        .await?;
        self.append_audit(
            &mut tx,
            "theme.settings_draft_saved",
            "theme",
            id.as_str(),
            serde_json::json!({"manifest_id": theme.manifest.id, "revision": revision}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Publishes the current settings draft.
    ///
    /// The draft is validated again against the theme schema, copied to the
    /// live settings and recorded as a published revision.
    pub async fn publish_settings(
        &self,
        id: &ThemeId,
        created_by: Option<String>,
        expected_revision: Option<i64>,
    ) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let theme = self.fetch_row_in(&mut tx, id).await?.into_theme()?;
        ensure_settings_revision(&theme, expected_revision)?;
        let draft = theme.draft_settings.clone().ok_or_else(|| {
            AppError::conflict(format!("theme has no settings draft to publish: {id}"))
        })?;
        validate_settings(theme.settings_schema.as_ref(), &draft)?;

        let settings_json = serialize_json(&draft, "settings")?;
        let revision = theme.settings_revision + 1;
        let now = now_timestamp();

        sqlx::query(
            "UPDATE themes SET settings = ?, draft_settings = NULL, settings_revision = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(&settings_json)
        .bind(revision)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.write_settings_revision(
            &mut tx,
            id,
            revision,
            &settings_json,
            ThemeSettingsRevisionStatus::Published,
            created_by.as_deref(),
            now,
        )
        .await?;
        self.append_audit(
            &mut tx,
            "theme.settings_published",
            "theme",
            id.as_str(),
            serde_json::json!({"manifest_id": theme.manifest.id, "revision": revision}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Lists recent settings revisions, newest first.
    pub async fn settings_revisions(
        &self,
        id: &ThemeId,
        limit: i64,
    ) -> Result<Vec<ThemeSettingsRevision>, AppError> {
        self.fetch_row(id).await?;
        let limit = limit.clamp(1, Pagination::MAX_LIMIT);
        let sql = format!(
            "SELECT {SETTINGS_REVISION_COLUMNS} FROM theme_settings_revisions \
             WHERE theme_id = ? ORDER BY revision DESC LIMIT ?"
        );
        let rows = sqlx::query_as::<_, ThemeSettingsRevisionRow>(&sql)
            .bind(id.as_str())
            .bind(limit)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        rows.into_iter()
            .map(ThemeSettingsRevisionRow::into_revision)
            .collect()
    }

    /// Restores one historical settings revision as a new published revision.
    ///
    /// Historical rows are immutable; restoring validates the snapshot against
    /// the current schema and appends a fresh revision.
    pub async fn restore_settings_revision(
        &self,
        id: &ThemeId,
        revision: i64,
        created_by: Option<String>,
        expected_revision: Option<i64>,
    ) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let theme = self.fetch_row_in(&mut tx, id).await?.into_theme()?;
        ensure_settings_revision(&theme, expected_revision)?;
        let sql = format!(
            "SELECT {SETTINGS_REVISION_COLUMNS} FROM theme_settings_revisions \
             WHERE theme_id = ? AND revision = ?"
        );
        let source = sqlx::query_as::<_, ThemeSettingsRevisionRow>(&sql)
            .bind(id.as_str())
            .bind(revision)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| {
                AppError::not_found("theme settings revision", format!("{id}@{revision}"))
            })?
            .into_revision()?;
        validate_settings(theme.settings_schema.as_ref(), &source.settings)?;

        let settings_json = serialize_json(&source.settings, "settings")?;
        let next_revision = theme.settings_revision + 1;
        let now = now_timestamp();
        sqlx::query(
            "UPDATE themes SET settings = ?, draft_settings = NULL, settings_revision = ?, \
             updated_at = ? WHERE id = ?",
        )
        .bind(&settings_json)
        .bind(next_revision)
        .bind(now)
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.write_settings_revision(
            &mut tx,
            id,
            next_revision,
            &settings_json,
            ThemeSettingsRevisionStatus::Published,
            created_by.as_deref(),
            now,
        )
        .await?;
        self.append_audit(
            &mut tx,
            "theme.settings_restored",
            "theme",
            id.as_str(),
            serde_json::json!({
                "manifest_id": theme.manifest.id,
                "restored_revision": revision,
                "revision": next_revision,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get(id).await
    }

    /// Activates a theme. Activating the active theme is a no-op.
    pub async fn activate(&self, id: &ThemeId) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await?;
        let target = self.fetch_row_in(&mut tx, id).await?.into_theme()?;

        if target.is_active {
            tx.commit().await.map_err(database_error)?;
            return Ok(target);
        }

        let previous_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM themes WHERE is_active = 1 LIMIT 1")
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;

        let now = now_timestamp();
        sqlx::query("UPDATE themes SET is_active = 0, updated_at = ? WHERE is_active = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE themes SET is_active = 1, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO theme_activations (id, theme_id, previous_theme_id, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(ThemeActivationId::new().as_str())
        .bind(id.as_str())
        .bind(previous_id.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "theme.activated",
            "theme",
            id.as_str(),
            serde_json::json!({"manifest_id": target.manifest.id}),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get(id).await
    }

    /// Lists recent activations, newest first, with theme metadata.
    pub async fn activation_history(
        &self,
        limit: i64,
    ) -> Result<Vec<ThemeActivationRecord>, AppError> {
        let limit = limit.clamp(1, Pagination::MAX_LIMIT);
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                i64,
            ),
        >(
            "SELECT a.id, a.theme_id, t.manifest_id, t.name, t.version, \
             a.previous_theme_id, a.created_at \
             FROM theme_activations a LEFT JOIN themes t ON t.id = a.theme_id \
             ORDER BY a.created_at DESC, a.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, theme_id, manifest_id, theme_name, theme_version, previous, created_at)| {
                    ThemeActivationRecord {
                        id: ThemeActivationId::from_string(id),
                        theme_id: ThemeId::from_string(theme_id),
                        manifest_id,
                        theme_name,
                        theme_version,
                        previous_theme_id: previous.map(ThemeId::from_string),
                        created_at,
                    }
                },
            )
            .collect())
    }

    /// Returns the theme that a rollback would activate.
    ///
    /// Callers validate this theme before committing the rollback so a bad
    /// previous theme cannot be restored blindly.
    pub async fn previous(&self) -> Result<Theme, AppError> {
        let id = self.previous_theme_id().await?;
        self.get(&id).await
    }

    async fn previous_theme_id(&self) -> Result<ThemeId, AppError> {
        let latest_previous: Option<Option<String>> = sqlx::query_scalar(
            "SELECT previous_theme_id FROM theme_activations \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let previous_id = latest_previous
            .ok_or_else(|| AppError::conflict("no theme activation to roll back"))?
            .ok_or_else(|| {
                AppError::conflict("the latest theme activation has no previous theme")
            })?;
        Ok(ThemeId::from_string(previous_id))
    }

    /// Rolls back the most recent activation to the theme active before it.
    pub async fn rollback(&self) -> Result<Theme, AppError> {
        let mut tx = self.db.begin().await?;
        let latest_previous: Option<Option<String>> = sqlx::query_scalar(
            "SELECT previous_theme_id FROM theme_activations \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        let previous_id = latest_previous
            .ok_or_else(|| AppError::conflict("no theme activation to roll back"))?
            .ok_or_else(|| {
                AppError::conflict("the latest theme activation has no previous theme")
            })?;
        let previous_id = ThemeId::from_string(previous_id);

        let previous_manifest_id: Option<String> =
            sqlx::query_scalar("SELECT manifest_id FROM themes WHERE id = ?")
                .bind(previous_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        let Some(previous_manifest_id) = previous_manifest_id else {
            return Err(AppError::conflict(format!(
                "previous theme no longer exists: {previous_id}"
            )));
        };

        let current_id: Option<String> =
            sqlx::query_scalar("SELECT id FROM themes WHERE is_active = 1 LIMIT 1")
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;

        let now = now_timestamp();
        sqlx::query("UPDATE themes SET is_active = 0, updated_at = ? WHERE is_active = 1")
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE themes SET is_active = 1, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(previous_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO theme_activations (id, theme_id, previous_theme_id, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(ThemeActivationId::new().as_str())
        .bind(previous_id.as_str())
        .bind(current_id.as_deref())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "theme.rolled_back",
            "theme",
            previous_id.as_str(),
            serde_json::json!({"manifest_id": previous_manifest_id}),
        )
        .await?;

        tx.commit().await.map_err(database_error)?;
        self.get(&previous_id).await
    }

    /// Deletes an inactive theme.
    ///
    /// The active check is part of the delete statement so a concurrent
    /// activation cannot slip between the check and the delete.
    pub async fn delete(&self, id: &ThemeId) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        // The latest activation remembers the theme that rollback would
        // restore; deleting it would leave rollback pointing at a missing row.
        let latest_previous: Option<Option<String>> = sqlx::query_scalar(
            "SELECT previous_theme_id FROM theme_activations \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?;
        if latest_previous.flatten().as_deref() == Some(id.as_str()) {
            return Err(AppError::conflict(
                "cannot delete a theme targeted by the latest activation rollback",
            ));
        }
        let result = sqlx::query("DELETE FROM themes WHERE id = ? AND is_active = 0")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            // Distinguish a missing theme from an active one without relying
            // on the pre-delete state.
            drop(tx);
            let theme = self.fetch_row(id).await?;
            if theme.is_active != 0 {
                return Err(AppError::conflict("cannot delete the active theme"));
            }
            return Err(AppError::not_found("theme", id));
        }
        self.append_audit(&mut tx, "theme.deleted", "theme", id.as_str(), Value::Null)
            .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

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

    async fn fetch_row(&self, id: &ThemeId) -> Result<ThemeRow, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?");
        sqlx::query_as::<_, ThemeRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("theme", id))
    }

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
    draft_settings: Option<String>,
    settings_revision: i64,
    is_active: i64,
    created_at: i64,
    updated_at: i64,
}

impl ThemeRow {
    fn into_theme(self) -> Result<Theme, AppError> {
        let manifest = ThemeManifest {
            id: self.manifest_id,
            name: self.name,
            version: self.version,
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
