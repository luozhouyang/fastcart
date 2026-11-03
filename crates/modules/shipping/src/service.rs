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
    CreateShippingOptionInput, CreateShippingProfileInput, CreateShippingRateInput,
    ResolvedShippingOption, SelectedShippingRate, ShippingOption, ShippingProfile, ShippingRate,
    UpdateShippingOptionInput, UpdateShippingProfileInput, UpdateShippingRateInput,
    normalize_province, rate_specificity, validate_amount, validate_shipping_name,
    validate_subtotal_range,
};

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
     is_enabled, metadata, created_at, updated_at";
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

/// Application service for shipping profiles, options and destination rates.
#[derive(Clone, Debug)]
pub struct ShippingService {
    db: StoreDatabase,
}

impl ShippingService {
    /// Creates a service over a migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

    // Profiles

    /// Creates a shipping profile.
    pub async fn create_profile(
        &self,
        input: CreateShippingProfileInput,
    ) -> Result<ShippingProfile, AppError> {
        validate_shipping_name(&input.name, "name")?;
        validate_metadata_option(input.metadata.as_ref())?;
        let profile_type = input.profile_type.unwrap_or_else(|| "default".to_owned());
        validate_length(&profile_type, "profile_type", 40)?;
        let now = now_timestamp();
        let id = ShippingProfileId::new();
        sqlx::query(
            "INSERT INTO shipping_profiles (id, name, type, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.name.trim())
        .bind(profile_type)
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_profile(&id).await
    }

    /// Fetches a shipping profile.
    pub async fn get_profile(&self, id: &ShippingProfileId) -> Result<ShippingProfile, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM shipping_profiles WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_profile(&row),
            None => Err(AppError::not_found("shipping profile", id)),
        }
    }

    /// Lists shipping profiles.
    pub async fn list_profiles(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<ShippingProfile>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shipping_profiles")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {PROFILE_COLUMNS} FROM shipping_profiles \
             ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let profiles = rows
            .iter()
            .map(row_to_profile)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(profiles, count, pagination))
    }

    /// Updates a shipping profile.
    pub async fn update_profile(
        &self,
        id: &ShippingProfileId,
        input: UpdateShippingProfileInput,
    ) -> Result<ShippingProfile, AppError> {
        let existing = self.get_profile(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_shipping_name(&name, "name")?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        let profile_type = match input.profile_type {
            Some(profile_type) => {
                validate_length(&profile_type, "profile_type", 40)?;
                profile_type
            }
            None => existing.profile_type,
        };
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        sqlx::query(
            "UPDATE shipping_profiles SET name = ?, type = ?, metadata = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(name)
        .bind(profile_type)
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_profile(id).await
    }

    /// Deletes a profile together with its options and rates.
    pub async fn delete_profile(&self, id: &ShippingProfileId) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM shipping_profiles WHERE id = ?")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("shipping profile", id));
        }
        Ok(())
    }

    // Options

    /// Creates a flat-price shipping option inside a profile.
    pub async fn create_option(
        &self,
        input: CreateShippingOptionInput,
    ) -> Result<ShippingOption, AppError> {
        validate_shipping_name(&input.name, "name")?;
        validate_amount(input.amount_minor)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let mut tx = self.db.begin().await?;
        ensure_profile_in_tx(&mut tx, &input.profile_id).await?;
        let now = now_timestamp();
        let id = ShippingOptionId::new();
        sqlx::query(
            "INSERT INTO shipping_options \
             (id, profile_id, name, price_type, amount, currency_code, is_enabled, metadata, \
              created_at, updated_at) \
             VALUES (?, ?, ?, 'flat', ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.profile_id.as_str())
        .bind(input.name.trim())
        .bind(input.amount_minor)
        .bind(input.currency.as_str())
        .bind(i64::from(input.is_enabled.unwrap_or(true)))
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        tx.commit().await.map_err(database_error)?;
        self.get_option(&id).await
    }

    /// Fetches a shipping option.
    pub async fn get_option(&self, id: &ShippingOptionId) -> Result<ShippingOption, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {OPTION_COLUMNS} FROM shipping_options WHERE id = ?"
        ))
        .bind(id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_option(&row),
            None => Err(AppError::not_found("shipping option", id)),
        }
    }

    /// Lists every shipping option.
    pub async fn list_options(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<ShippingOption>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shipping_options")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {OPTION_COLUMNS} FROM shipping_options \
             ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let options = rows
            .iter()
            .map(row_to_option)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(options, count, pagination))
    }

    /// Lists the options belonging to one profile.
    pub async fn list_options_for_profile(
        &self,
        profile_id: &ShippingProfileId,
        pagination: Pagination,
    ) -> Result<PageResult<ShippingOption>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM shipping_options WHERE profile_id = ?")
                .bind(profile_id.as_str())
                .fetch_one(self.db.pool())
                .await
                .map_err(database_error)?;
        let rows = sqlx::query(&format!(
            "SELECT {OPTION_COLUMNS} FROM shipping_options WHERE profile_id = ? \
             ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(profile_id.as_str())
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let options = rows
            .iter()
            .map(row_to_option)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(options, count, pagination))
    }

    /// Updates a shipping option.
    pub async fn update_option(
        &self,
        id: &ShippingOptionId,
        input: UpdateShippingOptionInput,
    ) -> Result<ShippingOption, AppError> {
        let existing = self.get_option(id).await?;
        let name = match input.name {
            Some(name) => {
                validate_shipping_name(&name, "name")?;
                name.trim().to_owned()
            }
            None => existing.name,
        };
        let amount_minor = input.amount_minor.unwrap_or(existing.amount_minor);
        validate_amount(amount_minor)?;
        let currency = input.currency.unwrap_or(existing.currency);
        let is_enabled = input.is_enabled.unwrap_or(existing.is_enabled);
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        sqlx::query(
            "UPDATE shipping_options SET name = ?, amount = ?, currency_code = ?, \
             is_enabled = ?, metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(amount_minor)
        .bind(currency.as_str())
        .bind(i64::from(is_enabled))
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_option(id).await
    }

    /// Deletes a shipping option together with its rates and locations.
    pub async fn delete_option(&self, id: &ShippingOptionId) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM shipping_options WHERE id = ?")
            .bind(id.as_str())
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("shipping option", id));
        }
        Ok(())
    }

    // Rates

    /// Creates a destination-specific rate for an option.
    pub async fn create_rate(
        &self,
        input: CreateShippingRateInput,
    ) -> Result<ShippingRate, AppError> {
        validate_amount(input.amount_minor)?;
        validate_subtotal_range(input.min_subtotal, input.max_subtotal)?;
        validate_metadata_option(input.metadata.as_ref())?;
        self.ensure_option_exists(&input.shipping_option_id).await?;
        let id = new_rate_id();
        let province_code = normalize_province(input.province_code.as_deref());
        sqlx::query(
            "INSERT INTO shipping_rates \
             (id, shipping_option_id, country_code, province_code, amount, currency_code, \
              min_subtotal, max_subtotal, metadata, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(input.shipping_option_id.as_str())
        .bind(input.country_code.as_ref().map(CountryCode::as_str))
        .bind(province_code)
        .bind(input.amount_minor)
        .bind(input.currency.as_str())
        .bind(input.min_subtotal)
        .bind(input.max_subtotal)
        .bind(metadata_string(input.metadata))
        .bind(now_timestamp())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_rate(&id).await
    }

    /// Fetches a shipping rate.
    pub async fn get_rate(&self, id: &str) -> Result<ShippingRate, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {RATE_COLUMNS} FROM shipping_rates WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_rate(&row),
            None => Err(AppError::not_found("shipping rate", id)),
        }
    }

    /// Lists the rates configured for an option.
    pub async fn list_rates(
        &self,
        option_id: &ShippingOptionId,
    ) -> Result<Vec<ShippingRate>, AppError> {
        self.ensure_option_exists(option_id).await?;
        let rows = sqlx::query(&format!(
            "SELECT {RATE_COLUMNS} FROM shipping_rates WHERE shipping_option_id = ? \
             ORDER BY id"
        ))
        .bind(option_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.iter().map(row_to_rate).collect()
    }

    /// Updates a shipping rate.
    pub async fn update_rate(
        &self,
        id: &str,
        input: UpdateShippingRateInput,
    ) -> Result<ShippingRate, AppError> {
        let existing = self.get_rate(id).await?;
        let country_code = match input.country_code {
            Some(country_code) => country_code,
            None => existing.country_code,
        };
        let province_code = match input.province_code {
            Some(province_code) => normalize_province(province_code.as_deref()),
            None => existing.province_code,
        };
        let amount_minor = input.amount_minor.unwrap_or(existing.amount_minor);
        validate_amount(amount_minor)?;
        let currency = input.currency.unwrap_or(existing.currency);
        let min_subtotal = match input.min_subtotal {
            Some(value) => value,
            None => existing.min_subtotal,
        };
        let max_subtotal = match input.max_subtotal {
            Some(value) => value,
            None => existing.max_subtotal,
        };
        validate_subtotal_range(min_subtotal, max_subtotal)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        sqlx::query(
            "UPDATE shipping_rates SET country_code = ?, province_code = ?, amount = ?, \
             currency_code = ?, min_subtotal = ?, max_subtotal = ?, metadata = ? WHERE id = ?",
        )
        .bind(country_code.as_ref().map(CountryCode::as_str))
        .bind(province_code)
        .bind(amount_minor)
        .bind(currency.as_str())
        .bind(min_subtotal)
        .bind(max_subtotal)
        .bind(metadata.to_json_string())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_rate(id).await
    }

    /// Deletes a shipping rate.
    pub async fn delete_rate(&self, id: &str) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM shipping_rates WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("shipping rate", id));
        }
        Ok(())
    }

    // Selection

    /// Selects the most specific rate for a destination and subtotal.
    ///
    /// Returns `None` when the option does not exist, is disabled, or is
    /// restricted to other countries. When no rate matches and the option has
    /// no country-specific rates, its flat amount is used as a fallback.
    pub async fn select_rate(
        &self,
        option_id: &ShippingOptionId,
        country_code: &CountryCode,
        province_code: Option<&str>,
        subtotal: i64,
    ) -> Result<Option<SelectedShippingRate>, AppError> {
        let row = sqlx::query(&format!(
            "SELECT {OPTION_COLUMNS} FROM shipping_options WHERE id = ?"
        ))
        .bind(option_id.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let option = row_to_option(&row)?;
        self.resolve_rate_for_option(&option, country_code, province_code, subtotal)
            .await
    }

    /// Lists enabled options with their resolved amount for a destination.
    pub async fn list_options_for(
        &self,
        country_code: &CountryCode,
        subtotal: i64,
    ) -> Result<Vec<ResolvedShippingOption>, AppError> {
        let rows = sqlx::query(&format!(
            "SELECT {OPTION_COLUMNS} FROM shipping_options WHERE is_enabled = 1 ORDER BY id"
        ))
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let options = rows
            .iter()
            .map(row_to_option)
            .collect::<Result<Vec<_>, _>>()?;
        let mut resolved = Vec::with_capacity(options.len());
        for option in options {
            let Some(selected) = self
                .resolve_rate_for_option(&option, country_code, None, subtotal)
                .await?
            else {
                continue;
            };
            resolved.push(ResolvedShippingOption {
                option,
                amount_minor: selected.amount_minor,
                currency: selected.currency,
                rate_id: selected.rate_id,
            });
        }
        Ok(resolved)
    }

    async fn resolve_rate_for_option(
        &self,
        option: &ShippingOption,
        country_code: &CountryCode,
        province_code: Option<&str>,
        subtotal: i64,
    ) -> Result<Option<SelectedShippingRate>, AppError> {
        if !option.is_enabled {
            return Ok(None);
        }
        let rows = sqlx::query(&format!(
            "SELECT {RATE_COLUMNS} FROM shipping_rates WHERE shipping_option_id = ?"
        ))
        .bind(option.id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let rates = rows
            .iter()
            .map(row_to_rate)
            .collect::<Result<Vec<_>, _>>()?;

        let mut best: Option<(u8, &ShippingRate)> = None;
        for rate in &rates {
            let Some(score) = rate_specificity(rate, country_code, province_code, subtotal) else {
                continue;
            };
            let replace = match &best {
                None => true,
                Some((best_score, best_rate)) => {
                    score > *best_score || (score == *best_score && rate.id < best_rate.id)
                }
            };
            if replace {
                best = Some((score, rate));
            }
        }

        if best.is_none() && rates.iter().any(|rate| rate.country_code.is_some()) {
            // Country-specific pricing means the option is not offered in a
            // destination without a matching rate.
            return Ok(None);
        }

        Ok(match best {
            Some((_, rate)) => Some(SelectedShippingRate {
                option_id: option.id.clone(),
                rate_id: Some(rate.id.clone()),
                amount_minor: rate.amount_minor,
                currency: rate.currency.clone(),
                is_fallback: false,
            }),
            None => Some(SelectedShippingRate {
                option_id: option.id.clone(),
                rate_id: None,
                amount_minor: option.amount_minor,
                currency: option.currency.clone(),
                is_fallback: true,
            }),
        })
    }

    // Locations

    /// Restricts an option to an inventory location.
    pub async fn assign_location(
        &self,
        option_id: &ShippingOptionId,
        location_id: &str,
    ) -> Result<(), AppError> {
        validate_length(location_id, "location_id", 120)?;
        self.ensure_option_exists(option_id).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO shipping_option_locations \
             (shipping_option_id, location_id) VALUES (?, ?)",
        )
        .bind(option_id.as_str())
        .bind(location_id.trim())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Removes a location restriction from an option.
    pub async fn remove_location(
        &self,
        option_id: &ShippingOptionId,
        location_id: &str,
    ) -> Result<(), AppError> {
        self.ensure_option_exists(option_id).await?;
        sqlx::query(
            "DELETE FROM shipping_option_locations \
             WHERE shipping_option_id = ? AND location_id = ?",
        )
        .bind(option_id.as_str())
        .bind(location_id.trim())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(())
    }

    /// Lists the inventory locations assigned to an option.
    pub async fn list_locations(
        &self,
        option_id: &ShippingOptionId,
    ) -> Result<Vec<String>, AppError> {
        self.ensure_option_exists(option_id).await?;
        let locations: Vec<String> = sqlx::query_scalar(
            "SELECT location_id FROM shipping_option_locations \
             WHERE shipping_option_id = ? ORDER BY location_id",
        )
        .bind(option_id.as_str())
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(locations)
    }

    async fn ensure_option_exists(&self, id: &ShippingOptionId) -> Result<(), AppError> {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM shipping_options WHERE id = ?")
                .bind(id.as_str())
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("shipping option", id));
        }
        Ok(())
    }
}
