//! Shared page and navigation row reads.

use super::*;

impl ContentService {
    pub(super) async fn fetch_page_row(
        &self,
        id: &PageId,
        include_deleted: bool,
    ) -> Result<PageRow, AppError> {
        let sql = if include_deleted {
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ?")
        } else {
            format!("SELECT {PAGE_COLUMNS} FROM pages WHERE id = ? AND deleted_at IS NULL")
        };
        sqlx::query_as::<_, PageRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("page", id))
    }
}

impl ContentService {
    pub(super) async fn ensure_page_handle_available(
        &self,
        handle: &str,
        exclude: Option<&PageId>,
    ) -> Result<(), AppError> {
        let exists: i64 = match exclude {
            Some(id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(id.as_str())
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM pages WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if exists > 0 {
            return Err(AppError::conflict(format!(
                "page handle already exists: {handle}"
            )));
        }
        Ok(())
    }
}

impl ContentService {
    pub(super) async fn fetch_navigation_row(
        &self,
        id: &NavigationId,
    ) -> Result<NavigationRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE id = ?");
        sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", id))
    }
}

impl ContentService {
    pub(super) async fn fetch_navigation_row_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        id: &NavigationId,
    ) -> Result<NavigationRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE id = ?");
        sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", id))
    }
}

impl ContentService {
    pub(super) async fn fetch_navigation_row_by_handle(
        &self,
        handle: &str,
    ) -> Result<NavigationRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_COLUMNS} FROM navigations WHERE handle = ?");
        sqlx::query_as::<_, NavigationRow>(&sql)
            .bind(handle)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation", handle))
    }
}

impl ContentService {
    pub(super) async fn ensure_navigation_handle_available(
        &self,
        handle: &str,
        exclude: Option<&NavigationId>,
    ) -> Result<(), AppError> {
        let exists: i64 = match exclude {
            Some(id) => {
                sqlx::query_scalar("SELECT COUNT(*) FROM navigations WHERE handle = ? AND id <> ?")
                    .bind(handle)
                    .bind(id.as_str())
                    .fetch_one(self.db.pool())
                    .await
                    .map_err(database_error)?
            }
            None => sqlx::query_scalar("SELECT COUNT(*) FROM navigations WHERE handle = ?")
                .bind(handle)
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?,
        };
        if exists > 0 {
            return Err(AppError::conflict(format!(
                "navigation handle already exists: {handle}"
            )));
        }
        Ok(())
    }
}

impl ContentService {
    pub(super) async fn items_for_navigation(
        &self,
        navigation_id: &NavigationId,
    ) -> Result<Vec<NavigationItem>, AppError> {
        let sql = format!(
            "SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE navigation_id = ? \
             ORDER BY rank ASC, id ASC"
        );
        let rows = sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(navigation_id.as_str())
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        Ok(rows.into_iter().map(NavigationItemRow::into_item).collect())
    }
}

impl ContentService {
    pub(super) async fn fetch_navigation_item_row(
        &self,
        item_id: &str,
    ) -> Result<NavigationItemRow, AppError> {
        let sql = format!("SELECT {NAVIGATION_ITEM_COLUMNS} FROM navigation_items WHERE id = ?");
        sqlx::query_as::<_, NavigationItemRow>(&sql)
            .bind(item_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("navigation item", item_id))
    }
}

impl ContentService {
    pub(super) async fn fetch_navigation_item(
        &self,
        item_id: &str,
    ) -> Result<NavigationItem, AppError> {
        self.fetch_navigation_item_row(item_id)
            .await
            .map(NavigationItemRow::into_item)
    }
}

impl ContentService {
    pub(super) fn validated_item_title(&self, title: &str) -> Result<String, AppError> {
        validate_length(title, "title", MAX_TITLE_LENGTH)?;
        Ok(title.trim().to_owned())
    }
}

impl ContentService {
    pub(super) fn validated_item_url(&self, url: &str) -> Result<String, AppError> {
        validate_length(url, "url", MAX_URL_LENGTH)?;
        Ok(url.trim().to_owned())
    }
}

impl ContentService {
    pub(super) async fn ensure_parent_in_navigation(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        navigation_id: &NavigationId,
        parent_id: &str,
        item_id: Option<&str>,
    ) -> Result<(), AppError> {
        if item_id == Some(parent_id) {
            return Err(AppError::validation(
                "navigation item cannot be its own parent",
            ));
        }

        let mut current = Some(parent_id.to_owned());
        let mut steps = 0_usize;
        while let Some(current_id) = current {
            if Some(current_id.as_str()) == item_id {
                return Err(AppError::validation(
                    "navigation item parent would create a cycle",
                ));
            }
            let row: Option<(String, Option<String>)> = sqlx::query_as(
                "SELECT navigation_id, parent_id FROM navigation_items WHERE id = ?",
            )
            .bind(&current_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
            let Some((owner, next_parent)) = row else {
                return Err(AppError::validation(
                    "navigation item parent does not exist",
                ));
            };
            if owner != navigation_id.as_str() {
                return Err(AppError::validation(
                    "navigation item parent must belong to the same navigation",
                ));
            }
            current = next_parent;
            steps += 1;
            if steps > 10_000 {
                return Err(AppError::validation(
                    "navigation item hierarchy is too deep",
                ));
            }
        }
        Ok(())
    }
}
