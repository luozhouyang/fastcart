//! Tax rates.

use super::*;

impl TaxService {
    /// Creates a tax rate for a region and optional category.
    ///
    /// A region may have at most one rate per category (including the
    /// uncategorized bucket). Setting a default clears the region's previous
    /// default inside the same transaction so exactly one default survives.
    pub async fn create_rate(&self, input: CreateTaxRateInput) -> Result<TaxRate, AppError> {
        validate_tax_name(&input.name, "name")?;
        validate_rate_bps(input.rate_bps)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let mut tx = self.db.begin().await?;
        self.ensure_region_in_tx(&mut tx, &input.tax_region_id)
            .await?;
        if let Some(category_id) = input.tax_category_id.as_ref() {
            ensure_category_in_tx(&mut tx, category_id).await?;
        }
        if input.is_default {
            clear_region_defaults_in_tx(&mut tx, &input.tax_region_id, None).await?;
        }
        let now = now_timestamp();
        let id = TaxRateId::new();
        sqlx::query(
            "INSERT INTO tax_rates \
             (id, tax_region_id, tax_category_id, name, rate_bps, is_inclusive, is_default, \
              metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.tax_region_id.as_str())
        .bind(input.tax_category_id.as_ref().map(TaxCategoryId::as_str))
        .bind(input.name.trim())
        .bind(input.rate_bps)
        .bind(i64::from(input.is_inclusive))
        .bind(i64::from(input.is_default))
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            unique_or_database(
                error,
                "a tax rate already exists for this region and category",
            )
        })?;
        tx.commit().await.map_err(database_error)?;
        self.get_rate(&id).await
    }
}

impl TaxService {
    /// Fetches a tax rate.
    pub async fn get_rate(&self, id: &TaxRateId) -> Result<TaxRate, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {RATE_COLUMNS} FROM tax_rates WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_rate(&row),
            None => Err(AppError::not_found("tax rate", id)),
        }
    }
}

impl TaxService {
    /// Lists the rates configured for a region.
    pub async fn list_rates(&self, region_id: &TaxRegionId) -> Result<Vec<TaxRate>, AppError> {
        self.ensure_region_exists(region_id).await?;
        let rows = sqlx::query(&format!(
            "SELECT {RATE_COLUMNS} FROM tax_rates WHERE tax_region_id = ? ORDER BY id"
        ))
        .bind(region_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.iter().map(row_to_rate).collect()
    }
}

impl TaxService {
    /// Updates a tax rate.
    ///
    /// The read, merge and write all happen under the `BEGIN IMMEDIATE` write
    /// lock so concurrent partial updates cannot lose each other's fields.
    pub async fn update_rate(
        &self,
        id: &TaxRateId,
        input: UpdateTaxRateInput,
    ) -> Result<TaxRate, AppError> {
        if let Some(name) = input.name.as_deref() {
            validate_tax_name(name, "name")?;
        }
        if let Some(rate_bps) = input.rate_bps {
            validate_rate_bps(rate_bps)?;
        }
        validate_metadata_option(input.metadata.as_ref())?;

        let mut tx = self.db.begin().await?;
        let existing = get_rate_in_tx(&mut tx, id).await?;
        let region_id = existing.tax_region_id.clone();
        let category_id = input.tax_category_id.unwrap_or(existing.tax_category_id);
        if let Some(category_id) = category_id.as_ref() {
            ensure_category_in_tx(&mut tx, category_id).await?;
        }
        let name = match input.name {
            Some(name) => name.trim().to_owned(),
            None => existing.name,
        };
        let rate_bps = input.rate_bps.unwrap_or(existing.rate_bps);
        let is_inclusive = input.is_inclusive.unwrap_or(existing.is_inclusive);
        let is_default = input.is_default.unwrap_or(existing.is_default);
        let metadata = input.metadata.unwrap_or(existing.metadata);

        if is_default {
            clear_region_defaults_in_tx(&mut tx, &region_id, Some(id)).await?;
        }

        let affected = sqlx::query(
            "UPDATE tax_rates SET tax_category_id = ?, name = ?, rate_bps = ?, \
             is_inclusive = ?, is_default = ?, metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(category_id.as_ref().map(TaxCategoryId::as_str))
        .bind(name)
        .bind(rate_bps)
        .bind(i64::from(is_inclusive))
        .bind(i64::from(is_default))
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            unique_or_database(
                error,
                "a tax rate already exists for this region and category",
            )
        })?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("tax rate", id));
        }
        tx.commit().await.map_err(database_error)?;
        self.get_rate(id).await
    }
}

impl TaxService {
    /// Deletes a tax rate.
    pub async fn delete_rate(&self, id: &TaxRateId) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM tax_rates WHERE id = ?")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("tax rate", id));
        }
        Ok(())
    }
}
