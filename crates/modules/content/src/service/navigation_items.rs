//! Navigation item mutation and ordering.

use super::*;

impl ContentService {
    /// Adds an item to a navigation.
    pub async fn add_navigation_item(
        &self,
        navigation_id: &NavigationId,
        input: NavigationItemInput,
    ) -> Result<NavigationItem, AppError> {
        let mut tx = self.db.begin().await?;
        self.fetch_navigation_row_in_tx(&mut tx, navigation_id)
            .await?;
        let title = self.validated_item_title(&input.title)?;
        let url = self.validated_item_url(&input.url)?;
        let rank = input.rank.unwrap_or(0);
        validate_rank(rank)?;
        if let Some(parent_id) = input.parent_id.as_deref() {
            self.ensure_parent_in_navigation(&mut tx, navigation_id, parent_id, None)
                .await?;
        }
        validate_metadata(&input.metadata)?;

        let id = generate_id("nitem");
        let inserted = sqlx::query(
            "INSERT INTO navigation_items \
             (id, navigation_id, parent_id, title, url, rank, metadata) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(navigation_id.as_str())
        .bind(input.parent_id.as_deref())
        .bind(&title)
        .bind(&url)
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if inserted.rows_affected() != 1 {
            return Err(AppError::internal("navigation item was not created"));
        }
        self.append_audit(
            &mut tx,
            "navigation_item.created",
            "navigation_item",
            &id,
            serde_json::json!({
                "navigation_id": navigation_id.as_str(),
                "title": title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.fetch_navigation_item(&id).await
    }
}

impl ContentService {
    /// Updates an existing navigation item.
    ///
    /// The item and its ancestor chain are read under the same write lock as
    /// the update, so the cycle check cannot race a concurrent reparent.
    pub async fn update_navigation_item(
        &self,
        item_id: &str,
        input: NavigationItemInput,
    ) -> Result<NavigationItem, AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE id = ?");
        let existing = sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation item", item_id))?;
        let navigation_id = NavigationId::from_string(existing.navigation_id.clone());
        let title = self.validated_item_title(&input.title)?;
        let url = self.validated_item_url(&input.url)?;
        let rank = input.rank.unwrap_or(existing.rank);
        validate_rank(rank)?;
        if let Some(parent_id) = input.parent_id.as_deref() {
            self.ensure_parent_in_navigation(&mut tx, &navigation_id, parent_id, Some(item_id))
                .await?;
        }
        validate_metadata(&input.metadata)?;

        let updated = sqlx::query(
            "UPDATE navigation_items SET parent_id = ?, title = ?, url = ?, rank = ?, \
             metadata = ? WHERE id = ?",
        )
        .bind(input.parent_id.as_deref())
        .bind(&title)
        .bind(&url)
        .bind(rank)
        .bind(input.metadata.to_json_string())
        .bind(item_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(AppError::not_found("navigation item", item_id));
        }
        self.append_audit(
            &mut tx,
            "navigation_item.updated",
            "navigation_item",
            item_id,
            serde_json::json!({
                "navigation_id": navigation_id.as_str(),
                "title": title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.fetch_navigation_item(item_id).await
    }
}

impl ContentService {
    /// Deletes an item and, through foreign keys, all descendant items.
    pub async fn remove_navigation_item(&self, item_id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let sql = format!("SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE id = ?");
        let row = sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation item", item_id))?;
        sqlx::query("DELETE FROM navigation_items WHERE id = ?")
            .bind(item_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "navigation_item.deleted",
            "navigation_item",
            item_id,
            serde_json::json!({
                "navigation_id": row.navigation_id,
                "title": row.title,
            }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}

impl ContentService {
    /// Reorders navigation items by assigning ranks from the given order.
    pub async fn reorder_navigation_items(
        &self,
        navigation_id: &NavigationId,
        item_ids: &[String],
    ) -> Result<Vec<NavigationItem>, AppError> {
        self.fetch_navigation_row(navigation_id).await?;
        let mut seen = HashSet::new();
        for item_id in item_ids {
            if !seen.insert(item_id.as_str()) {
                return Err(AppError::validation(
                    "navigation item ids must be unique when reordering",
                ));
            }
        }

        let mut tx = self.db.begin().await?;
        for (index, item_id) in item_ids.iter().enumerate() {
            let owner: Option<String> =
                sqlx::query_scalar("SELECT navigation_id FROM navigation_items WHERE id = ?")
                    .bind(item_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_error)?;
            match owner {
                Some(owner) if owner == navigation_id.as_str() => {}
                Some(_) => {
                    return Err(AppError::validation(
                        "navigation item must belong to the same navigation",
                    ));
                }
                None => return Err(AppError::not_found("navigation item", item_id)),
            }
            let rank = i64::try_from(index)
                .map_err(|error| AppError::internal(format!("rank overflow: {error}")))?;
            sqlx::query("UPDATE navigation_items SET rank = ? WHERE id = ?")
                .bind(rank)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
        }
        self.append_audit(
            &mut tx,
            "navigation.items_reordered",
            "navigation",
            navigation_id.as_str(),
            serde_json::json!({ "item_ids": item_ids }),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;

        self.items_for_navigation(navigation_id).await
    }
}
