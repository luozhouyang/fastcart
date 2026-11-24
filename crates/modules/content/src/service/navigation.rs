//! Navigation authoring and tree reads.

use super::*;

impl ContentService {
    /// Creates a navigation menu. The handle defaults to a slug of the name.
    pub async fn create_navigation(
        &self,
        input: CreateNavigationInput,
    ) -> Result<Navigation, AppError> {
        let CreateNavigationInput {
            name,
            handle,
            metadata,
        } = input;
        validate_length(&name, "name", MAX_TITLE_LENGTH)?;
        let name = name.trim().to_owned();
        let handle = resolve_handle(&name, handle)?;
        validate_metadata(&metadata)?;
        self.ensure_navigation_handle_available(&handle, None)
            .await?;

        let id = NavigationId::new();
        let now = now_timestamp();
        let mut tx = self.db.begin().await?;
        let conflict_message = format!("navigation handle already exists: {handle}");
        let inserted = sqlx::query(
            "INSERT INTO navigations (id, name, handle, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(&name)
        .bind(&handle)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, &conflict_message))?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::internal("navigation was not created"));
        }
        self.append_audit(
            &mut tx,
            "navigation.created",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": handle, "name": name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_navigation(&id).await
    }
}

impl ContentService {
    /// Loads a navigation with its flat, rank-ordered items.
    pub async fn get_navigation(&self, id: &NavigationId) -> Result<Navigation, AppError> {
        let row = self.fetch_navigation_row(id).await?;
        let items = self.items_for_navigation(id).await?;
        Ok(row.into_navigation(items))
    }
}

impl ContentService {
    /// Loads a navigation by handle with its flat, rank-ordered items.
    pub async fn get_navigation_by_handle(&self, handle: &str) -> Result<Navigation, AppError> {
        let row = self.fetch_navigation_row_by_handle(handle).await?;
        let id = NavigationId::from_string(row.id.clone());
        let items = self.items_for_navigation(&id).await?;
        Ok(row.into_navigation(items))
    }
}

impl ContentService {
    /// Loads a navigation by handle with items nested by parent.
    pub async fn get_navigation_tree(&self, handle: &str) -> Result<NavigationTree, AppError> {
        let navigation = self.get_navigation_by_handle(handle).await?;
        Ok(navigation_tree_from(navigation))
    }
}

impl ContentService {
    /// Loads a navigation by identifier with items nested by parent.
    ///
    /// This performs a primary-key lookup before building the tree, avoiding a
    /// second handle lookup when the identifier is already known.
    pub async fn get_navigation_tree_by_id(
        &self,
        id: &NavigationId,
    ) -> Result<NavigationTree, AppError> {
        let navigation = self.get_navigation(id).await?;
        Ok(navigation_tree_from(navigation))
    }
}

impl ContentService {
    /// Lists navigation menus with their items.
    pub async fn list_navigations(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Navigation>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM navigations")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let sql = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigations ORDER BY created_at DESC, id DESC \
             LIMIT ? OFFSET ?"
        );
        let rows = sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let id = NavigationId::from_string(row.id.clone());
            let items = self.items_for_navigation(&id).await?;
            data.push(row.into_navigation(items));
        }
        Ok(PageResult::new(data, count, pagination))
    }
}

impl ContentService {
    /// Searches navigations by name or handle, newest first.
    ///
    /// The query is matched case-insensitively and LIKE wildcards in user
    /// input are escaped. An absent or blank query matches every navigation.
    pub async fn search_navigations(
        &self,
        query: Option<&str>,
        pagination: Pagination,
    ) -> Result<PageResult<Navigation>, AppError> {
        let pattern = like_pattern(query);
        let conditions = if pattern.is_some() {
            " WHERE LOWER(name) LIKE LOWER(?) ESCAPE '\\' \
             OR LOWER(handle) LIKE LOWER(?) ESCAPE '\\'"
        } else {
            ""
        };

        let count_sql = format!("SELECT COUNT(*) FROM navigations{conditions}");
        let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(pattern) = pattern.as_deref() {
            count_query = count_query.bind(pattern).bind(pattern);
        }
        let count = count_query
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let list_sql = format!(
            "SELECT {NAVIGATION_COLUMNS} FROM navigations{conditions} \
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
        );
        let mut list_query = sqlx::query_as::<_, NavigationRow>(&list_sql);
        if let Some(pattern) = pattern.as_deref() {
            list_query = list_query.bind(pattern).bind(pattern);
        }
        let rows = list_query
            .bind(pagination.limit)
            .bind(pagination.offset)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let id = NavigationId::from_string(row.id.clone());
            let items = self.items_for_navigation(&id).await?;
            data.push(row.into_navigation(items));
        }
        Ok(PageResult::new(data, count, pagination))
    }
}

impl ContentService {
    /// Updates navigation fields. Omitted fields keep their current value.
    pub async fn update_navigation(
        &self,
        id: &NavigationId,
        input: UpdateNavigationInput,
    ) -> Result<Navigation, AppError> {
        let existing = self.fetch_navigation_row(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_length(&name, "name", MAX_TITLE_LENGTH)?;
                name.trim().to_owned()
            }
            None => existing.name.clone(),
        };
        let handle = match input.handle {
            Some(handle) => {
                validate_handle(&handle, "handle")?;
                handle
            }
            None => existing.handle.clone(),
        };
        self.ensure_navigation_handle_available(&handle, Some(id))
            .await?;
        let metadata = match input.metadata {
            Some(metadata) => metadata,
            None => Metadata::from_json_str(&existing.metadata),
        };
        validate_metadata(&metadata)?;

        let mut tx = self.db.begin().await?;
        let conflict_message = format!("navigation handle already exists: {handle}");
        let updated = sqlx::query(
            "UPDATE navigations SET name = ?, handle = ?, metadata = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(&handle)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| unique_conflict(error, &conflict_message))?;
        if updated.rows_affected() != 1 {
            return Err(AppError::not_found("navigation", id));
        }
        self.append_audit(
            &mut tx,
            "navigation.updated",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": handle, "name": name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.get_navigation(id).await
    }
}

impl ContentService {
    /// Deletes a navigation and, through foreign keys, all of its items.
    pub async fn delete_navigation(&self, id: &NavigationId) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE id = ?");
        let row = sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", id))?;
        sqlx::query("DELETE FROM navigations WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation.deleted",
            "navigation",
            id.as_str(),
            serde_json::json!({ "handle": row.handle, "name": row.name }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
