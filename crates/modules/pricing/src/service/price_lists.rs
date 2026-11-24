//! Price-list authoring and listing.

use super::*;

impl PricingService {
    /// Creates a price list together with its rules.
    pub async fn create_price_list(
        &self,
        input: CreatePriceListInput,
    ) -> Result<PriceList, AppError> {
        validate_price_list_fields(
            &input.title,
            input.starts_at,
            input.ends_at,
            &input.metadata,
        )?;
        let id = PriceListId::new().to_string();
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;

        sqlx::query(
            "INSERT INTO price_lists
             (id, title, description, type, status, starts_at, ends_at, metadata, created_at,
              updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.title)
        .bind(&input.description)
        .bind(input.price_type.as_str())
        .bind(input.status.as_str())
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;

        let rules = insert_price_list_rules(&mut transaction, &id, &input.rules, now).await?;
        transaction.commit().await.map_err(database_error)?;

        Ok(PriceList {
            id,
            title: input.title,
            description: input.description,
            price_type: input.price_type,
            status: input.status,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            metadata: input.metadata,
            rules,
            created_at: now,
            updated_at: now,
        })
    }
}

impl PricingService {
    /// Updates a price list and optionally replaces its rules.
    ///
    /// The row is read and rewritten under one `BEGIN IMMEDIATE` transaction
    /// so concurrent partial updates cannot overwrite each other.
    pub async fn update_price_list(
        &self,
        id: &str,
        input: UpdatePriceListInput,
    ) -> Result<PriceList, AppError> {
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let existing = load_price_list_tx(&mut transaction, id).await?;
        let created_at = existing.created_at;
        let existing_rules = existing.rules;

        let title = input.title.unwrap_or(existing.title);
        let description = input.description.unwrap_or(existing.description);
        let price_type = input.price_type.unwrap_or(existing.price_type);
        let status = input.status.unwrap_or(existing.status);
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        let metadata = input.metadata.unwrap_or(existing.metadata);

        validate_price_list_fields(&title, starts_at, ends_at, &metadata)?;

        let now = now_timestamp();
        let affected = sqlx::query(
            "UPDATE price_lists
             SET title = ?, description = ?, type = ?, status = ?, starts_at = ?, ends_at = ?,
                 metadata = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&description)
        .bind(price_type.as_str())
        .bind(status.as_str())
        .bind(starts_at)
        .bind(ends_at)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("price list", id));
        }

        let rules = match input.rules {
            Some(rules) => {
                sqlx::query("DELETE FROM price_list_rules WHERE price_list_id = ?")
                    .bind(id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(database_error)?;
                insert_price_list_rules(&mut transaction, id, &rules, now).await?
            }
            None => existing_rules,
        };
        transaction.commit().await.map_err(database_error)?;

        Ok(PriceList {
            id: id.to_owned(),
            title,
            description,
            price_type,
            status,
            starts_at,
            ends_at,
            metadata,
            rules,
            created_at,
            updated_at: now,
        })
    }
}

impl PricingService {
    /// Loads one price list with its rules.
    /// Deletes a price list and its rules. Prices referencing the list are
    /// deleted by the foreign-key cascade.
    pub async fn delete_price_list(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM price_lists WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("price list", id));
        }
        Ok(())
    }
}

impl PricingService {
    pub async fn get_price_list(&self, id: &str) -> Result<PriceList, AppError> {
        let row = sqlx::query_as::<_, PriceListRow>(&format!(
            "SELECT {PRICE_LIST_COLUMNS} FROM price_lists WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("price list", id))?;

        let mut price_list = row.into_price_list()?;
        price_list.rules = self.load_price_list_rules(&price_list.id).await?;
        Ok(price_list)
    }
}

impl PricingService {
    /// Lists price lists ordered by creation time.
    pub async fn list_price_lists(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<PriceList>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM price_lists")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query_as::<_, PriceListRow>(&format!(
            "SELECT {PRICE_LIST_COLUMNS} FROM price_lists ORDER BY created_at, id LIMIT ? OFFSET ?"
        ))
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let mut price_lists = Vec::with_capacity(rows.len());
        for row in rows {
            let mut price_list = row.into_price_list()?;
            price_list.rules = self.load_price_list_rules(&price_list.id).await?;
            price_lists.push(price_list);
        }
        Ok(PageResult::new(price_lists, count, pagination))
    }
}
