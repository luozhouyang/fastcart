//! Theme settings editing and revisions.

use super::*;

impl ThemeService {
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
}

impl ThemeService {
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
}

impl ThemeService {
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
}

impl ThemeService {
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
}

impl ThemeService {
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
}
