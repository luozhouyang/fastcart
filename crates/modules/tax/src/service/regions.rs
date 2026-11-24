//! Tax regions and destination matching.

use super::*;

impl TaxService {
    /// Creates a tax region, rejecting duplicate country/province pairs.
    pub async fn create_region(&self, input: CreateTaxRegionInput) -> Result<TaxRegion, AppError> {
        validate_tax_name(&input.name, "name")?;
        validate_metadata_option(input.metadata.as_ref())?;
        let province_code = normalize_province(input.province_code.as_deref());
        let mut tx = self.db.begin().await?;
        if region_exists_in_tx(&mut tx, &input.country_code, province_code.as_deref(), None).await?
        {
            return Err(AppError::conflict(
                "a tax region already exists for this country and province",
            ));
        }
        let now = now_timestamp();
        let id = TaxRegionId::new();
        sqlx::query(
            "INSERT INTO tax_regions \
             (id, name, country_code, province_code, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(input.country_code.as_str())
        .bind(province_code)
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            unique_or_database(
                error,
                "a tax region already exists for this country and province",
            )
        })?;
        tx.commit().await.map_err(database_error)?;
        self.get_region(&id).await
    }
}

impl TaxService {
    /// Fetches a tax region.
    pub async fn get_region(&self, id: &TaxRegionId) -> Result<TaxRegion, AppError> {
        fetch_region(&self.db, id).await
    }
}

impl TaxService {
    /// Lists tax regions.
    pub async fn list_regions(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<TaxRegion>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tax_regions")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {REGION_COLUMNS} FROM tax_regions ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let regions = rows
            .iter()
            .map(row_to_region)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(regions, count, pagination))
    }
}

impl TaxService {
    /// Updates a tax region.
    pub async fn update_region(
        &self,
        id: &TaxRegionId,
        input: UpdateTaxRegionInput,
    ) -> Result<TaxRegion, AppError> {
        let mut tx = self.db.begin().await?;
        let existing = get_region_in_tx(&mut tx, id).await?;
        let name = match input.name {
            Some(name) => {
                validate_tax_name(&name, "name")?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        let country_code = input.country_code.unwrap_or(existing.country_code);
        let province_code = match input.province_code {
            Some(province_code) => normalize_province(province_code.as_deref()),
            None => existing.province_code,
        };
        if region_exists_in_tx(&mut tx, &country_code, province_code.as_deref(), Some(id)).await? {
            return Err(AppError::conflict(
                "a tax region already exists for this country and province",
            ));
        }
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        let affected = sqlx::query(
            "UPDATE tax_regions SET name = ?, country_code = ?, province_code = ?, \
             metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(country_code.as_str())
        .bind(province_code)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            unique_or_database(
                error,
                "a tax region already exists for this country and province",
            )
        })?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("tax region", id));
        }
        tx.commit().await.map_err(database_error)?;
        self.get_region(id).await
    }
}

impl TaxService {
    /// Deletes a tax region together with its rates.
    pub async fn delete_region(&self, id: &TaxRegionId) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM tax_regions WHERE id = ?")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("tax region", id));
        }
        Ok(())
    }
}

impl TaxService {
    pub(crate) async fn ensure_region_exists(&self, id: &TaxRegionId) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM tax_regions WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("tax region", id));
        }
        Ok(())
    }
}

impl TaxService {
    pub(crate) async fn ensure_region_in_tx(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        id: &TaxRegionId,
    ) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM tax_regions WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("tax region", id));
        }
        Ok(())
    }
}
