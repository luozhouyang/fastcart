//! SQLite-backed shipping application service.

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CountryCode, CurrencyCode, Metadata, PageResult, Pagination, ShippingOptionId,
    ShippingProfileId, now_timestamp, validate_length, validate_metadata,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::domain::{
    CreateFulfillmentSetInput, CreateShippingOptionInput, CreateShippingProfileInput,
    CreateShippingRateInput, FulfillmentSet, ResolvedShippingOption, SelectedShippingRate,
    ShippingOption, ShippingProfile, ShippingRate, UpdateFulfillmentSetInput,
    UpdateShippingOptionInput, UpdateShippingProfileInput, UpdateShippingRateInput,
    normalize_province, rate_specificity, validate_amount, validate_shipping_name,
    validate_subtotal_range,
};

mod options;
mod profiles;
mod rates;

impl ShippingService {
    /// Creates a service over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    // Profiles
}

fn new_rate_id() -> String {
    format!("shrate_{}", Uuid::now_v7().simple())
}

fn metadata_string(metadata: Option<Metadata>) -> String {
    metadata.unwrap_or_default().to_json_string()
}

fn validate_metadata_option(metadata: Option<&Metadata>) -> Result<(), AppError> {
    if let Some(metadata) = metadata {
        validate_metadata(metadata)?;
    }
    Ok(())
}

fn currency_from_row(row: &SqliteRow) -> Result<CurrencyCode, AppError> {
    let code: String = row.try_get("currency_code").map_err(database_error)?;
    Ok(CurrencyCode::new(code)?)
}

fn row_to_profile(row: &SqliteRow) -> Result<ShippingProfile, AppError> {
    Ok(ShippingProfile {
        id: ShippingProfileId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        name: row.try_get("name").map_err(database_error)?,
        profile_type: row.try_get("profile_type").map_err(database_error)?,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_option(row: &SqliteRow) -> Result<ShippingOption, AppError> {
    Ok(ShippingOption {
        id: ShippingOptionId::from_string(row.try_get::<String, _>("id").map_err(database_error)?),
        profile_id: ShippingProfileId::from_string(
            row.try_get::<String, _>("profile_id")
                .map_err(database_error)?,
        ),
        name: row.try_get("name").map_err(database_error)?,
        price_type: row.try_get("price_type").map_err(database_error)?,
        amount_minor: row.try_get("amount").map_err(database_error)?,
        currency: currency_from_row(row)?,
        provider_id: row.try_get("provider_id").map_err(database_error)?,
        provider_data: serde_json::from_str(
            &row.try_get::<String, _>("provider_data")
                .map_err(database_error)?,
        )
        .map_err(|error| AppError::internal(format!("invalid provider_data: {error}")))?,
        fulfillment_set_id: row.try_get("fulfillment_set_id").map_err(database_error)?,
        is_enabled: row
            .try_get::<i64, _>("is_enabled")
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

fn row_to_fulfillment_set(row: &SqliteRow) -> Result<FulfillmentSet, AppError> {
    Ok(FulfillmentSet {
        id: row.try_get("id").map_err(database_error)?,
        name: row.try_get("name").map_err(database_error)?,
        set_type: row.try_get("set_type").map_err(database_error)?,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_rate(row: &SqliteRow) -> Result<ShippingRate, AppError> {
    let country_code = row
        .try_get::<Option<String>, _>("country_code")
        .map_err(database_error)?
        .map(CountryCode::new)
        .transpose()?;
    Ok(ShippingRate {
        id: row.try_get("id").map_err(database_error)?,
        shipping_option_id: ShippingOptionId::from_string(
            row.try_get::<String, _>("shipping_option_id")
                .map_err(database_error)?,
        ),
        country_code,
        province_code: row.try_get("province_code").map_err(database_error)?,
        amount_minor: row.try_get("amount").map_err(database_error)?,
        currency: currency_from_row(row)?,
        min_subtotal: row.try_get("min_subtotal").map_err(database_error)?,
        max_subtotal: row.try_get("max_subtotal").map_err(database_error)?,
        metadata: Metadata::from_json_str(
            &row.try_get::<String, _>("metadata")
                .map_err(database_error)?,
        ),
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

const PROFILE_COLUMNS: &str = "id, name, type AS profile_type, metadata, created_at, updated_at";
const OPTION_COLUMNS: &str = "id, profile_id, name, price_type, amount, currency_code, \
     provider_id, provider_data, fulfillment_set_id, is_enabled, metadata, created_at, \
     updated_at";
const RATE_COLUMNS: &str = "id, shipping_option_id, country_code, province_code, amount, \
     currency_code, min_subtotal, max_subtotal, metadata, created_at";

async fn ensure_profile_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &ShippingProfileId,
) -> Result<(), AppError> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM shipping_profiles WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if existing.is_none() {
        return Err(AppError::not_found("shipping profile", id));
    }
    Ok(())
}

async fn ensure_fulfillment_set_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<(), AppError> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM fulfillment_sets WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_error)?;
    if existing.is_none() {
        return Err(AppError::not_found("fulfillment set", id));
    }
    Ok(())
}

/// Application service for shipping profiles, options and destination rates.
#[derive(Clone, Debug)]
pub struct ShippingService {
    db: StoreDatabase,
}
