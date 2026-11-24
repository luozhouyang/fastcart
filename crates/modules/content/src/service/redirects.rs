//! Redirect management.

use super::*;

impl ContentService {
    /// Creates a redirect.
    pub async fn create_redirect(&self, input: CreateRedirectInput) -> Result<Redirect, AppError> {
        let from_path = input.from_path.trim().to_owned();
        if !from_path.starts_with('/') {
            return Err(AppError::validation("from_path must start with '/'"));
        }
        validate_length(&from_path, "from_path", MAX_REDIRECT_PATH_LENGTH)?;
        let to_path = input.to_path.trim().to_owned();
        validate_length(&to_path, "to_path", MAX_REDIRECT_PATH_LENGTH)?;
        let status_code = input.status_code.unwrap_or(301);
        validate_redirect_status(status_code)?;

        let mut tx = self.db.begin().await?;
        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM redirects WHERE from_path = ?")
                .bind(&from_path)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if existing > 0 {
            return Err(AppError::conflict(format!(
                "redirect already exists for: {from_path}"
            )));
        }

        let id = generate_id("redirect");
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO redirects (id, from_path, to_path, status_code, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&from_path)
        .bind(&to_path)
        .bind(status_code)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "redirect.created",
            "redirect",
            &id,
            serde_json::json!({
                "from_path": from_path,
                "to_path": to_path,
                "status_code": status_code,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_redirect(&id).await
    }
}

impl ContentService {
    /// Loads a redirect by identifier.
    pub async fn get_redirect(&self, id: &str) -> Result<Redirect, AppError> {
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", id))?;
        Ok(row.into_redirect())
    }
}

impl ContentService {
    /// Loads a redirect by its source path.
    pub async fn get_redirect_by_path(&self, from_path: &str) -> Result<Redirect, AppError> {
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE from_path = ?",
        )
        .bind(from_path)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", from_path))?;
        Ok(row.into_redirect())
    }
}

impl ContentService {
    /// Lists redirects with pagination.
    pub async fn list_redirects(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Redirect>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM redirects")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let data = rows.into_iter().map(RedirectRow::into_redirect).collect();
        Ok(PageResult::new(data, count, pagination))
    }
}

impl ContentService {
    /// Deletes a redirect.
    pub async fn delete_redirect(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let row = sqlx::query_as::<_, RedirectRow>(
            "SELECT id, from_path, to_path, status_code, created_at FROM redirects \
             WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("redirect", id))?;
        sqlx::query("DELETE FROM redirects WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "redirect.deleted",
            "redirect",
            id,
            serde_json::json!({
                "from_path": row.from_path,
                "to_path": row.to_path,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
