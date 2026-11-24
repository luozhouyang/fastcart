//! Shipping options and location assignment.

use super::*;

impl ShippingService {
    /// Creates a flat-price shipping option inside a profile.
    pub async fn create_option(
        &self,
        input: CreateShippingOptionInput,
    ) -> Result<ShippingOption, AppError> {
        validate_shipping_name(&input.name, "name")?;
        validate_amount(input.amount_minor)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let provider_id = input.provider_id.unwrap_or_else(|| "manual".to_owned());
        validate_length(provider_id.trim(), "provider_id", 64)?;
        let provider_data = input
            .provider_data
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        if !provider_data.is_object() {
            return Err(AppError::validation("provider_data must be a JSON object"));
        }
        let mut tx = self.db.begin().await?;
        ensure_profile_in_tx(&mut tx, &input.profile_id).await?;
        if let Some(set_id) = input.fulfillment_set_id.as_deref() {
            ensure_fulfillment_set_in_tx(&mut tx, set_id).await?;
        }
        let now = now_timestamp();
        let id = ShippingOptionId::new();
        sqlx::query(
            "INSERT INTO shipping_options \
             (id, profile_id, name, price_type, amount, currency_code, provider_id, \
              provider_data, fulfillment_set_id, is_enabled, metadata, created_at, updated_at) \
             VALUES (?, ?, ?, 'flat', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.as_str())
        .bind(input.profile_id.as_str())
        .bind(input.name.trim())
        .bind(input.amount_minor)
        .bind(input.currency.as_str())
        .bind(provider_id.trim())
        .bind(provider_data.to_string())
        .bind(input.fulfillment_set_id.as_deref())
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
        let provider_id = input.provider_id.unwrap_or(existing.provider_id);
        validate_length(provider_id.trim(), "provider_id", 64)?;
        let provider_data = input.provider_data.unwrap_or(existing.provider_data);
        if !provider_data.is_object() {
            return Err(AppError::validation("provider_data must be a JSON object"));
        }
        let fulfillment_set_id = input
            .fulfillment_set_id
            .unwrap_or(existing.fulfillment_set_id);
        if let Some(set_id) = fulfillment_set_id.as_deref() {
            let exists: Option<String> =
                sqlx::query_scalar("SELECT id FROM fulfillment_sets WHERE id = ?")
                    .bind(set_id)
                    .fetch_optional(self.db.pool())
                    .await
                    .map_err(database_error)?;
            if exists.is_none() {
                return Err(AppError::not_found("fulfillment set", set_id));
            }
        }
        let is_enabled = input.is_enabled.unwrap_or(existing.is_enabled);
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        sqlx::query(
            "UPDATE shipping_options SET name = ?, amount = ?, currency_code = ?, \
             provider_id = ?, provider_data = ?, fulfillment_set_id = ?, is_enabled = ?, \
             metadata = ?, updated_at = ? WHERE id = ?",
        )
        .bind(name)
        .bind(amount_minor)
        .bind(currency.as_str())
        .bind(provider_id.trim())
        .bind(provider_data.to_string())
        .bind(fulfillment_set_id.as_deref())
        .bind(i64::from(is_enabled))
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id.as_str())
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_option(id).await
    }
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
    pub(crate) async fn ensure_option_exists(&self, id: &ShippingOptionId) -> Result<(), AppError> {
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
