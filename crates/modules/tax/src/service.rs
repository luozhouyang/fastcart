//! SQLite-backed tax application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CountryCode, Metadata, PageResult, Pagination, TaxCategoryId, TaxRateId, TaxRegionId,
    Timestamp, now_timestamp, validate_metadata,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::domain::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxBreakdown, TaxCategory,
    TaxLine, TaxRate, TaxRegion, UpdateTaxCategoryInput, UpdateTaxRateInput, UpdateTaxRegionInput,
    calculate_tax, normalize_province, validate_category_code, validate_rate_bps,
    validate_tax_name,
};

const CATEGORY_COLUMNS: &str = "id, name, code, description, created_at, updated_at";
const REGION_COLUMNS: &str =
    "id, name, country_code, province_code, metadata, created_at, updated_at";
const RATE_COLUMNS: &str = "id, tax_region_id, tax_category_id, name, rate_bps, is_inclusive, \
     is_default, metadata, created_at, updated_at";

fn metadata_string(metadata: Option<Metadata>) -> String {
    metadata.unwrap_or_default().to_json_string()
}

fn validate_metadata_option(metadata: Option<&Metadata>) -> Result<(), AppError> {
    if let Some(metadata) = metadata {
        validate_metadata(metadata)?;
    }
    Ok(())
}

fn row_to_category(row: &SqliteRow) -> Result<TaxCategory, AppError> {
    Ok(TaxCategory {
        id: TaxCategoryId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        name: row.try_get("name").map_err(database_error)?,
        code: row.try_get("code").map_err(database_error)?,
        description: row.try_get("description").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_region(row: &SqliteRow) -> Result<TaxRegion, AppError> {
    let country_code: String = row.try_get("country_code").map_err(database_error)?;
    Ok(TaxRegion {
        id: TaxRegionId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        name: row.try_get("name").map_err(database_error)?,
        country_code: CountryCode::new(country_code)?,
        province_code: row.try_get("province_code").map_err(database_error)?,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_rate(row: &SqliteRow) -> Result<TaxRate, AppError> {
    let category_id = row
        .try_get::<Option<String>, _>("tax_category_id")
        .map_err(database_error)?
        .map(TaxCategoryId::from_string);
    Ok(TaxRate {
        id: TaxRateId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        tax_region_id: TaxRegionId::from_string(
            row.try_get::<String, _>("tax_region_id")
                .map_err(database_error)?,
        ),
        tax_category_id: category_id,
        name: row.try_get("name").map_err(database_error)?,
        rate_bps: row.try_get("rate_bps").map_err(database_error)?,
        is_inclusive: row
            .try_get::<i64, _>("is_inclusive")
            .map_err(database_error)?
            != 0,
        is_default: row
            .try_get::<i64, _>("is_default")
            .map_err(database_error)?
            != 0,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

/// Picks the most recently created rate, using the identifier as a stable
/// tie-break for rates created within the same second.
fn newest_rate<'a>(rates: impl Iterator<Item = &'a TaxRate>) -> Option<&'a TaxRate> {
    rates.max_by(|left, right| {
        (left.created_at, left.id.as_str()).cmp(&(right.created_at, right.id.as_str()))
    })
}

fn pick_rate(rates: &[TaxRate], category_id: Option<&TaxCategoryId>) -> Option<TaxRate> {
    if let Some(category_id) = category_id
        && let Some(rate) = newest_rate(
            rates
                .iter()
                .filter(|rate| rate.tax_category_id.as_ref() == Some(category_id)),
        )
    {
        return Some(rate.clone());
    }
    if let Some(rate) = newest_rate(rates.iter().filter(|rate| rate.is_default)) {
        return Some(rate.clone());
    }
    newest_rate(rates.iter().filter(|rate| rate.tax_category_id.is_none())).cloned()
}

async fn get_region_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &TaxRegionId,
) -> Result<TaxRegion, AppError> {
    let row = sqlx::query(&format!(
        "SELECT {REGION_COLUMNS} FROM tax_regions WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    match row {
        Some(row) => row_to_region(&row),
        None => Err(AppError::not_found("tax region", id)),
    }
}

async fn get_rate_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &TaxRateId,
) -> Result<TaxRate, AppError> {
    let row = sqlx::query(&format!(
        "SELECT {RATE_COLUMNS} FROM tax_rates WHERE id = ?"
    ))
    .bind(id.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    match row {
        Some(row) => row_to_rate(&row),
        None => Err(AppError::not_found("tax rate", id)),
    }
}

async fn region_exists_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    country_code: &CountryCode,
    province_code: Option<&str>,
    exclude: Option<&TaxRegionId>,
) -> Result<bool, AppError> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM tax_regions WHERE country_code = ? \
         AND IFNULL(province_code, '') = IFNULL(?, '') AND (? IS NULL OR id <> ?)",
    )
    .bind(country_code.as_str())
    .bind(province_code)
    .bind(exclude.map(TaxRegionId::as_str))
    .bind(exclude.map(TaxRegionId::as_str))
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(existing.is_some())
}

async fn clear_region_defaults_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    region_id: &TaxRegionId,
    exclude: Option<&TaxRateId>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE tax_rates SET is_default = 0 \
         WHERE tax_region_id = ? AND is_default <> 0 AND (? IS NULL OR id <> ?)",
    )
    .bind(region_id.as_str())
    .bind(exclude.map(TaxRateId::as_str))
    .bind(exclude.map(TaxRateId::as_str))
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn ensure_category_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &TaxCategoryId,
) -> Result<(), AppError> {
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM tax_categories WHERE id = ?")
        .bind(id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;
    if existing.is_none() {
        return Err(AppError::not_found("tax category", id));
    }
    Ok(())
}

/// Application service for tax categories, regions and rates.
#[derive(Clone, Debug)]
pub struct TaxService {
    db: StoreDatabase,
}

impl TaxService {
    /// Creates a service over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    // Categories

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

    // Regions

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

    /// Fetches a tax region.
    pub async fn get_region(&self, id: &TaxRegionId) -> Result<TaxRegion, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {REGION_COLUMNS} FROM tax_regions WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_region(&row),
            None => Err(AppError::not_found("tax region", id)),
        }
    }

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

    // Rates

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

    // Resolution and calculation

    /// Resolves the effective rate and region for a destination and category.
    ///
    /// An exact country+province region is preferred over a country-only
    /// region. Within a region a category-specific rate wins over a default
    /// rate, which wins over an uncategorized rate. When `at` is provided, only
    /// rates created at or before that timestamp are considered.
    pub async fn resolve_rate(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
        category_id: Option<&TaxCategoryId>,
        at: Option<Timestamp>,
    ) -> Result<Option<(TaxRate, TaxRegion)>, AppError> {
        let province_code = normalize_province(province_code);
        let rows = sqlx::query(&format!(
            "SELECT {REGION_COLUMNS} FROM tax_regions WHERE country_code = ?"
        ))
        .bind(country_code.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let regions = rows
            .iter()
            .map(row_to_region)
            .collect::<Result<Vec<_>, _>>()?;

        let mut ordered = Vec::new();
        if province_code.is_some()
            && let Some(region) = regions
                .iter()
                .find(|region| region.province_code == province_code)
        {
            ordered.push(region.clone());
        }
        if let Some(region) = regions.iter().find(|region| region.province_code.is_none()) {
            ordered.push(region.clone());
        }

        for region in ordered {
            let rates = self.rates_for_region(&region.id, at).await?;
            if let Some(rate) = pick_rate(&rates, category_id) {
                return Ok(Some((rate, region)));
            }
        }
        Ok(None)
    }

    /// Calculates tax for one line, optionally overriding inclusive pricing.
    pub async fn calculate_for_line(
        &self,
        country_code: &CountryCode,
        province_code: Option<&str>,
        category_id: Option<&TaxCategoryId>,
        amount_minor: i64,
        inclusive_override: Option<bool>,
    ) -> Result<TaxLine, AppError> {
        if amount_minor < 0 {
            return Err(AppError::validation("amount_minor must not be negative"));
        }
        let resolved = self
            .resolve_rate(country_code, province_code, category_id, None)
            .await?;
        let (rate, region, rate_bps, is_inclusive, breakdown) = match resolved {
            Some((rate, region)) => {
                let is_inclusive = inclusive_override.unwrap_or(rate.is_inclusive);
                let breakdown = calculate_tax(amount_minor, rate.rate_bps, is_inclusive);
                (
                    Some(rate.clone()),
                    Some(region),
                    rate.rate_bps,
                    is_inclusive,
                    breakdown,
                )
            }
            None => {
                let breakdown = TaxBreakdown {
                    tax_amount: 0,
                    net_amount: amount_minor,
                    gross_amount: amount_minor,
                };
                (None, None, 0, false, breakdown)
            }
        };
        Ok(TaxLine {
            country_code: country_code.clone(),
            province_code: normalize_province(province_code),
            category_id: category_id.cloned(),
            rate,
            region,
            amount_minor,
            rate_bps,
            is_inclusive,
            breakdown,
        })
    }

    async fn rates_for_region(
        &self,
        region_id: &TaxRegionId,
        at: Option<Timestamp>,
    ) -> Result<Vec<TaxRate>, AppError> {
        let rows = match at {
            Some(at) => {
                sqlx::query(&format!(
                    "SELECT {RATE_COLUMNS} FROM tax_rates \
                     WHERE tax_region_id = ? AND created_at <= ? ORDER BY id"
                ))
                .bind(region_id.as_str())
                .bind(at)
                .fetch_all(self.db.pool())
                .await
            }
            None => {
                sqlx::query(&format!(
                    "SELECT {RATE_COLUMNS} FROM tax_rates WHERE tax_region_id = ? ORDER BY id"
                ))
                .bind(region_id.as_str())
                .fetch_all(self.db.pool())
                .await
            }
        }
        .map_err(database_error)?;
        rows.iter().map(row_to_rate).collect()
    }

    async fn ensure_region_exists(&self, id: &TaxRegionId) -> Result<(), AppError> {
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

    async fn ensure_region_in_tx(
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
