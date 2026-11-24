//! SQLite-backed tax application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CountryCode, Metadata, PageResult, Pagination, TaxCategoryId, TaxRateId, TaxRegionId,
    Timestamp, now_timestamp, validate_metadata,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};

use crate::domain::{
    CreateTaxCategoryInput, CreateTaxRateInput, CreateTaxRegionInput, TaxCategory, TaxLine,
    TaxRate, TaxRegion, UpdateTaxCategoryInput, UpdateTaxRateInput, UpdateTaxRegionInput,
    normalize_province, validate_category_code, validate_rate_bps, validate_tax_name,
};
use crate::provider::{
    TaxCalculationRequest, TaxDestination, TaxProviderRegistry, fetch_region,
    region_for_destination, resolve_rate,
};

const CATEGORY_COLUMNS: &str = "id, name, code, description, created_at, updated_at";
pub(crate) const REGION_COLUMNS: &str =
    "id, name, country_code, province_code, metadata, created_at, updated_at";
pub(crate) const RATE_COLUMNS: &str = "id, tax_region_id, tax_category_id, name, rate_bps, \
     is_inclusive, is_default, metadata, created_at, updated_at";

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

pub(crate) fn row_to_region(row: &SqliteRow) -> Result<TaxRegion, AppError> {
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

pub(crate) fn row_to_rate(row: &SqliteRow) -> Result<TaxRate, AppError> {
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
///
/// Tax calculations run through the active [`TaxProviderRegistry`] entry.
/// [`Self::new`] installs the built-in
/// [`RegionRateTaxProvider`](crate::provider::RegionRateTaxProvider), which
/// applies the regions and rates stored in this database; use
/// [`Self::with_registry`] to serve calculations from another provider.
#[derive(Clone, Debug)]
pub struct TaxService {
    db: StoreDatabase,
    providers: TaxProviderRegistry,
}

mod calculation;
mod categories;
mod rates;
mod regions;

impl TaxService {
    /// Creates a service over a migrated store database.
    ///
    /// Calculations use the built-in region/rate provider, which reads the
    /// regions and rates stored in `db`.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        let providers = TaxProviderRegistry::with_defaults(db.clone());
        Self::with_registry(db, providers)
    }
}

impl TaxService {
    /// Creates a service that calculates through the given provider registry.
    ///
    /// The registry determines which provider serves
    /// [`Self::calculate_for_line`] and [`Self::calculate_for_line_in_region`];
    /// it must have an active provider, otherwise calculations fail with
    /// [`AppError::Internal`]. Only the calculation paths are provider-backed:
    /// region and rate administration always operates on the service's own
    /// database.
    #[must_use]
    pub fn with_registry(db: StoreDatabase, providers: TaxProviderRegistry) -> Self {
        Self { db, providers }
    }
}

impl TaxService {
    /// Returns the registry that serves tax calculations.
    #[must_use]
    pub fn providers(&self) -> &TaxProviderRegistry {
        &self.providers
    }
}
