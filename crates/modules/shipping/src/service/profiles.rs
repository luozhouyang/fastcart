//! Profiles and fulfillment sets.

use super::*;

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
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
}

impl ShippingService {
    /// Creates a fulfillment set.
    pub async fn create_fulfillment_set(
        &self,
        input: CreateFulfillmentSetInput,
    ) -> Result<FulfillmentSet, AppError> {
        validate_shipping_name(&input.name, "name")?;
        let set_type = input.set_type.unwrap_or_else(|| "shipping".to_owned());
        validate_length(&set_type, "type", 64)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let id = format!("fset_{}", Uuid::now_v7().simple());
        let now = now_timestamp();
        sqlx::query(
            "INSERT INTO fulfillment_sets (id, name, type, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(input.name.trim())
        .bind(set_type.trim())
        .bind(metadata_string(input.metadata))
        .bind(now)
        .bind(now)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_fulfillment_set(&id).await
    }
}

impl ShippingService {
    /// Loads a fulfillment set.
    pub async fn get_fulfillment_set(&self, id: &str) -> Result<FulfillmentSet, AppError> {
        let row = sqlx::query(
            "SELECT id, name, type AS set_type, metadata, created_at, updated_at
             FROM fulfillment_sets WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("fulfillment set", id))?;
        row_to_fulfillment_set(&row)
    }
}

impl ShippingService {
    /// Lists fulfillment sets ordered by name.
    pub async fn list_fulfillment_sets(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<FulfillmentSet>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fulfillment_sets")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT id, name, type AS set_type, metadata, created_at, updated_at
             FROM fulfillment_sets ORDER BY name, id LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let sets = rows
            .iter()
            .map(row_to_fulfillment_set)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PageResult::new(sets, count, pagination))
    }
}

impl ShippingService {
    /// Partially updates a fulfillment set.
    pub async fn update_fulfillment_set(
        &self,
        id: &str,
        input: UpdateFulfillmentSetInput,
    ) -> Result<FulfillmentSet, AppError> {
        let existing = self.get_fulfillment_set(id).await?;
        let name = input.name.unwrap_or(existing.name);
        validate_shipping_name(&name, "name")?;
        let set_type = input.set_type.unwrap_or(existing.set_type);
        validate_length(&set_type, "type", 64)?;
        validate_metadata_option(input.metadata.as_ref())?;
        let metadata = input.metadata.unwrap_or(existing.metadata);
        sqlx::query(
            "UPDATE fulfillment_sets SET name = ?, type = ?, metadata = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(name.trim())
        .bind(set_type.trim())
        .bind(metadata.to_json_string())
        .bind(now_timestamp())
        .bind(id)
        .execute(self.db.pool())
        .await
        .map_err(database_error)?;
        self.get_fulfillment_set(id).await
    }
}

impl ShippingService {
    /// Deletes a fulfillment set; options referencing it keep existing but lose
    /// their set association.
    pub async fn delete_fulfillment_set(&self, id: &str) -> Result<(), AppError> {
        let affected = sqlx::query("DELETE FROM fulfillment_sets WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?
            .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("fulfillment set", id));
        }
        Ok(())
    }
}
