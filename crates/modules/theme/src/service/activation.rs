//! Theme activation, history and rollback.

use super::*;

impl ThemeService {
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
}

impl ThemeService {
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
}

impl ThemeService {
    /// Returns the theme that a rollback would activate.
    ///
    /// Callers validate this theme before committing the rollback so a bad
    /// previous theme cannot be restored blindly.
    pub async fn previous(&self) -> Result<Theme, AppError> {
        let id = self.previous_theme_id().await?;
        self.get(&id).await
    }
}

impl ThemeService {
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
}

impl ThemeService {
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
}
