//! Rates and rate selection.

use super::*;

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
    pub(crate) async fn resolve_rate_for_option(
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
}
