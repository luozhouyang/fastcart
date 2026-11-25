//! Theme registration, listing and deletion.

use super::*;

impl ThemeService {
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
        let requires_json = match manifest.requires.as_ref() {
            Some(requires) => Some(serialize_json(requires, "requires")?),
            None => None,
        };

        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO themes \
             (id, manifest_id, name, version, settings_schema, settings, requires_json, is_active, \
              created_at, updated_at, draft_settings, settings_revision) \
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, ?, NULL, 1)",
        )
        .bind(id.as_str())
        .bind(&manifest.id)
        .bind(&manifest.name)
        .bind(&manifest.version)
        .bind(settings_schema_json.as_deref())
        .bind(&settings_json)
        .bind(requires_json.as_deref())
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
}

impl ThemeService {
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
}

impl ThemeService {
    /// Loads a theme by internal identifier.
    pub async fn get(&self, id: &ThemeId) -> Result<Theme, AppError> {
        self.fetch_row(id).await?.into_theme()
    }
}

impl ThemeService {
    /// Returns the active theme, if one has been activated.
    pub async fn active(&self) -> Result<Option<Theme>, AppError> {
        let sql = format!("SELECT {THEME_COLUMNS} FROM themes WHERE is_active = 1 LIMIT 1");
        let row = sqlx::query_as::<_, ThemeRow>(&sql)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        row.map(ThemeRow::into_theme).transpose()
    }
}

impl ThemeService {
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
}
