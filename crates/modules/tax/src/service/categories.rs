//! Tax categories.

use super::*;

impl TaxService {
    /// Creates a tax category with a unique code.
    pub async fn create_category(
        &self,
        input: CreateTaxCategoryInput,
    ) -> Result<TaxCategory, AppError> {
        validate_tax_name(&input.name, "name")?;
        validate_category_code(&input.code)?;
        let code = input.code.trim().to_owned();
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM tax_categories WHERE code = ?")
                .bind(&code)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_some() {
            return Err(AppError::conflict(format!(
                "tax code already in use: {code}"
            )));
        }
        let now = now_timestamp();
        let id = TaxCategoryId::new();
        sqlx::query(
            "INSERT INTO tax_categories (id, name, code, description, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(&code)
        .bind(input.description)
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, &format!("tax code already in use: {code}")))?;
        self.get_category(&id).await
    }
}

impl TaxService {
    /// Fetches a tax category.
    pub async fn get_category(&self, id: &TaxCategoryId) -> Result<TaxCategory, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {CATEGORY_COLUMNS} FROM tax_categories WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_category(&row),
            None => Err(AppError::not_found("tax category", id)),
        }
    }
}

impl TaxService {
    /// Lists tax categories.
    pub async fn list_categories(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<TaxCategory>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tax_categories")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {CATEGORY_COLUMNS} FROM tax_categories \
             ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let categories = rows
            .iter()
            .map(row_to_category)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(categories, count, pagination))
    }
}

impl TaxService {
    /// Updates a tax category.
    pub async fn update_category(
        &self,
        id: &TaxCategoryId,
        input: UpdateTaxCategoryInput,
    ) -> Result<TaxCategory, AppError> {
        let existing = self.get_category(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_tax_name(&name, "name")?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        let code = match input.code {
            Some(code) => {
                validate_category_code(&code)?;
                let code = code.trim().to_owned();
                let duplicate: Option<String> =
                    sqlx::query_scalar("SELECT id FROM tax_categories WHERE code = ? AND id <> ?")
                        .bind(&code)
                        .bind(id.as_str())
                        .fetch_optional(self.db.pool())
                        .await
                        .map_err(database_error)?;
                if duplicate.is_some() {
                    return Err(AppError::conflict(format!(
                        "tax code already in use: {code}"
                    )));
                }
                code
            }
            None => existing.code,
        };
        let description = match input.description {
            Some(description) => description,
            None => existing.description,
        };
        sqlx::query(
            "UPDATE tax_categories SET name = ?, code = ?, description = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(name)
        .bind(code)
        .bind(description)
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(|error| unique_or_database(error, "tax code already in use"))?;
        self.get_category(id).await
    }
}

impl TaxService {
    /// Deletes a tax category.
    ///
    /// Categories referenced by tax rates or product variants are rejected
    /// rather than leaving dangling identifiers behind, because those columns
    /// intentionally have no foreign key.
    pub async fn delete_category(&self, id: &TaxCategoryId) -> Result<(), AppError> {
        let mut tx = self.db.begin().await?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM tax_categories WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("tax category", id));
        }
        let rate_references: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM tax_rates WHERE tax_category_id = ?")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        let variant_references: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE tax_category_id = ?")
                .bind(id.as_str())
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
        if rate_references > 0 || variant_references > 0 {
            return Err(AppError::conflict(
                "tax category is still referenced by tax rates or product variants",
            ));
        }
        let affected = sqlx::query("DELETE FROM tax_categories WHERE id = ?")
            .bind(id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("tax category", id));
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }
}
