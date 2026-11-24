//! Price authoring, bulk upsert and reads.

use super::*;

impl PricingService {
    /// Returns the price set linked to a variant, creating it when absent.
    pub async fn get_or_create_price_set_for_variant(
        &self,
        variant_id: &str,
    ) -> Result<PriceSet, AppError> {
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let price_set = ensure_price_set_tx(&mut transaction, variant_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(price_set)
    }
}

impl PricingService {
    /// Returns the price set linked to a variant, when one exists.
    pub async fn get_price_set_for_variant(
        &self,
        variant_id: &str,
    ) -> Result<Option<PriceSet>, AppError> {
        let row = sqlx::query_as::<_, PriceSetRow>(
            "SELECT ps.id, ps.title, ps.metadata, ps.created_at, ps.updated_at
             FROM price_sets ps
             JOIN product_variant_price_set link ON link.price_set_id = ps.id
             WHERE link.variant_id = ?",
        )
        .bind(variant_id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        row.map(PriceSetRow::into_price_set).transpose()
    }
}

impl PricingService {
    /// Creates a price.
    ///
    /// Blank customer-group and sales-channel scopes are normalized to `None`
    /// and non-blank scopes must reference existing rows.
    pub async fn create_price(&self, input: CreatePriceInput) -> Result<Price, AppError> {
        let customer_group_id = normalize_scope(input.customer_group_id.as_deref());
        let sales_channel_id = normalize_scope(input.sales_channel_id.as_deref());
        validate_price_fields(
            &input.variant_id,
            input.amount,
            input.min_quantity,
            input.max_quantity,
            input.starts_at,
            input.ends_at,
            &input.metadata,
        )?;

        let id = PriceId::new().to_string();
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let price_list_type = match input.price_list_id.as_deref() {
            Some(price_list_id) => {
                Some(ensure_price_list_exists_tx(&mut transaction, price_list_id).await?)
            }
            None => None,
        };
        ensure_scope_ids_exist_tx(
            &mut transaction,
            customer_group_id.as_deref(),
            sales_channel_id.as_deref(),
        )
        .await?;
        if find_price_id_by_scope_tx(
            &mut transaction,
            &input.variant_id,
            &input.currency_code,
            input.price_list_id.as_deref(),
            customer_group_id.as_deref(),
            sales_channel_id.as_deref(),
            input.min_quantity,
        )
        .await?
        .is_some()
        {
            return Err(AppError::conflict(
                "a price already exists for the same scope",
            ));
        }

        let price_set = ensure_price_set_tx(&mut transaction, &input.variant_id).await?;
        sqlx::query(
            "INSERT INTO prices
             (id, price_list_id, price_set_id, variant_id, currency_code, amount, min_quantity,
              max_quantity, customer_group_id, sales_channel_id, starts_at, ends_at, metadata,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&input.price_list_id)
        .bind(&price_set.id)
        .bind(&input.variant_id)
        .bind(input.currency_code.as_str())
        .bind(input.amount)
        .bind(input.min_quantity)
        .bind(input.max_quantity)
        .bind(&customer_group_id)
        .bind(&sales_channel_id)
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "a price already exists for the same scope"))?;
        transaction.commit().await.map_err(database_error)?;

        Ok(Price {
            id,
            price_list_id: input.price_list_id,
            price_list_type,
            variant_id: input.variant_id,
            currency_code: input.currency_code,
            amount: input.amount,
            min_quantity: input.min_quantity,
            max_quantity: input.max_quantity,
            customer_group_id,
            sales_channel_id,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
            price_list_rule_count: 0,
        })
    }
}

impl PricingService {
    /// Updates a price.
    ///
    /// The read, merge, validation and write happen under one `BEGIN
    /// IMMEDIATE` transaction, and blank scopes are normalized to `None`.
    pub async fn update_price(&self, id: &str, input: UpdatePriceInput) -> Result<Price, AppError> {
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let existing = get_price_tx(&mut transaction, id).await?;
        let created_at = existing.created_at;
        let price_list_rule_count = existing.price_list_rule_count;

        let price_list_id = input.price_list_id.unwrap_or(existing.price_list_id);
        let variant_id = input.variant_id.unwrap_or(existing.variant_id);
        let currency_code = input.currency_code.unwrap_or(existing.currency_code);
        let amount = input.amount.unwrap_or(existing.amount);
        let min_quantity = input.min_quantity.unwrap_or(existing.min_quantity);
        let max_quantity = input.max_quantity.unwrap_or(existing.max_quantity);
        let customer_group_id = match input.customer_group_id {
            Some(value) => normalize_scope(value.as_deref()),
            None => normalize_scope(existing.customer_group_id.as_deref()),
        };
        let sales_channel_id = match input.sales_channel_id {
            Some(value) => normalize_scope(value.as_deref()),
            None => normalize_scope(existing.sales_channel_id.as_deref()),
        };
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        let metadata = input.metadata.unwrap_or(existing.metadata);

        validate_price_fields(
            &variant_id,
            amount,
            min_quantity,
            max_quantity,
            starts_at,
            ends_at,
            &metadata,
        )?;
        let price_list_type = match price_list_id.as_deref() {
            Some(price_list_id) => {
                Some(ensure_price_list_exists_tx(&mut transaction, price_list_id).await?)
            }
            None => None,
        };
        ensure_scope_ids_exist_tx(
            &mut transaction,
            customer_group_id.as_deref(),
            sales_channel_id.as_deref(),
        )
        .await?;
        if let Some(found_id) = find_price_id_by_scope_tx(
            &mut transaction,
            &variant_id,
            &currency_code,
            price_list_id.as_deref(),
            customer_group_id.as_deref(),
            sales_channel_id.as_deref(),
            min_quantity,
        )
        .await?
            && found_id != id
        {
            return Err(AppError::conflict(
                "a price already exists for the same scope",
            ));
        }

        let now = now_timestamp();
        let price_set = ensure_price_set_tx(&mut transaction, &variant_id).await?;
        let affected = sqlx::query(
            "UPDATE prices
             SET price_list_id = ?, price_set_id = ?, variant_id = ?, currency_code = ?,
                 amount = ?, min_quantity = ?, max_quantity = ?, customer_group_id = ?,
                 sales_channel_id = ?, starts_at = ?, ends_at = ?, metadata = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(&price_list_id)
        .bind(&price_set.id)
        .bind(&variant_id)
        .bind(currency_code.as_str())
        .bind(amount)
        .bind(min_quantity)
        .bind(max_quantity)
        .bind(&customer_group_id)
        .bind(&sales_channel_id)
        .bind(starts_at)
        .bind(ends_at)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "a price already exists for the same scope"))?
        .rows_affected();
        if affected == 0 {
            return Err(AppError::not_found("price", id));
        }
        transaction.commit().await.map_err(database_error)?;

        Ok(Price {
            id: id.to_owned(),
            price_list_id,
            price_list_type,
            variant_id,
            currency_code,
            amount,
            min_quantity,
            max_quantity,
            customer_group_id,
            sales_channel_id,
            starts_at,
            ends_at,
            metadata,
            created_at,
            updated_at: now,
            price_list_rule_count,
        })
    }
}

impl PricingService {
    /// Deletes a price.
    pub async fn delete_price(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM prices WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("price", id));
        }
        Ok(())
    }
}

impl PricingService {
    /// Loads one price.
    pub async fn get_price(&self, id: &str) -> Result<Price, AppError> {
        let row = sqlx::query_as::<_, PriceRow>(&format!(
            "SELECT {PRICE_COLUMNS} FROM prices WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("price", id))?;
        row.into_price()
    }
}

impl PricingService {
    /// Lists prices with optional variant, price list and currency filters.
    pub async fn list_prices(
        &self,
        filter: PriceFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Price>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;

        let mut count_builder: QueryBuilder<'_, Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM prices WHERE 1 = 1");
        push_price_filters(&mut count_builder, &filter);
        let count = count_builder
            .build_query_scalar::<i64>()
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut builder: QueryBuilder<'_, Sqlite> =
            QueryBuilder::new(format!("SELECT {PRICE_COLUMNS} FROM prices WHERE 1 = 1"));
        push_price_filters(&mut builder, &filter);
        builder
            .push(" ORDER BY created_at, id LIMIT ")
            .push_bind(pagination.limit)
            .push(" OFFSET ")
            .push_bind(pagination.offset);
        let rows = builder
            .build_query_as::<PriceRow>()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let prices = rows
            .into_iter()
            .map(PriceRow::into_price)
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(PageResult::new(prices, count, pagination))
    }
}

impl PricingService {
    /// Inserts or updates a batch of prices keyed by their unique scope.
    ///
    /// Blank scopes are normalized before the duplicate-scope check, and
    /// non-blank scope identifiers must exist.
    pub async fn bulk_upsert_prices(
        &self,
        mut inputs: Vec<CreatePriceInput>,
    ) -> Result<Vec<Price>, AppError> {
        for input in &mut inputs {
            input.customer_group_id = normalize_scope(input.customer_group_id.as_deref());
            input.sales_channel_id = normalize_scope(input.sales_channel_id.as_deref());
            validate_price_fields(
                &input.variant_id,
                input.amount,
                input.min_quantity,
                input.max_quantity,
                input.starts_at,
                input.ends_at,
                &input.metadata,
            )?;
        }
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let now = now_timestamp();
        let mut ids = Vec::with_capacity(inputs.len());
        let mut seen_scopes = std::collections::HashSet::new();

        for input in inputs {
            if let Some(price_list_id) = input.price_list_id.as_deref() {
                ensure_price_list_exists_tx(&mut transaction, price_list_id).await?;
            }
            ensure_scope_ids_exist_tx(
                &mut transaction,
                input.customer_group_id.as_deref(),
                input.sales_channel_id.as_deref(),
            )
            .await?;
            let scope = scope_key(
                &input.variant_id,
                &input.currency_code,
                input.price_list_id.as_deref(),
                input.customer_group_id.as_deref(),
                input.sales_channel_id.as_deref(),
                input.min_quantity,
            );
            if !seen_scopes.insert(scope) {
                return Err(AppError::conflict(
                    "duplicate price scope in the same batch",
                ));
            }

            let existing_id: Option<String> = sqlx::query_scalar(
                "SELECT id FROM prices
                 WHERE variant_id = ? AND currency_code = ?
                   AND IFNULL(price_list_id, '') = ? AND IFNULL(customer_group_id, '') = ?
                   AND IFNULL(sales_channel_id, '') = ? AND min_quantity = ?
                 LIMIT 1",
            )
            .bind(&input.variant_id)
            .bind(input.currency_code.as_str())
            .bind(input.price_list_id.as_deref().unwrap_or(""))
            .bind(input.customer_group_id.as_deref().unwrap_or(""))
            .bind(input.sales_channel_id.as_deref().unwrap_or(""))
            .bind(input.min_quantity)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;

            let id = if let Some(existing_id) = existing_id {
                let price_set = ensure_price_set_tx(&mut transaction, &input.variant_id).await?;
                sqlx::query(
                    "UPDATE prices
                     SET max_quantity = ?, starts_at = ?, ends_at = ?, metadata = ?,
                         amount = ?, price_set_id = ?, updated_at = ?
                     WHERE id = ?",
                )
                .bind(input.max_quantity)
                .bind(input.starts_at)
                .bind(input.ends_at)
                .bind(input.metadata.to_json_string())
                .bind(input.amount)
                .bind(&price_set.id)
                .bind(now)
                .bind(&existing_id)
                .execute(&mut *transaction)
                .await
                .map_err(|error| {
                    unique_or_database(error, "a price already exists for the same scope")
                })?;
                existing_id
            } else {
                let id = PriceId::new().to_string();
                let price_set = ensure_price_set_tx(&mut transaction, &input.variant_id).await?;
                sqlx::query(
                    "INSERT INTO prices
                     (id, price_list_id, price_set_id, variant_id, currency_code, amount,
                      min_quantity, max_quantity, customer_group_id, sales_channel_id, starts_at,
                      ends_at, metadata, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&id)
                .bind(&input.price_list_id)
                .bind(&price_set.id)
                .bind(&input.variant_id)
                .bind(input.currency_code.as_str())
                .bind(input.amount)
                .bind(input.min_quantity)
                .bind(input.max_quantity)
                .bind(&input.customer_group_id)
                .bind(&input.sales_channel_id)
                .bind(input.starts_at)
                .bind(input.ends_at)
                .bind(input.metadata.to_json_string())
                .bind(now)
                .bind(now)
                .execute(&mut *transaction)
                .await
                .map_err(|error| {
                    unique_or_database(error, "a price already exists for the same scope")
                })?;
                id
            };
            ids.push(id);
        }
        transaction.commit().await.map_err(database_error)?;

        self.load_prices_by_ids(&ids).await
    }
}

impl PricingService {
    /// Loads several prices at once, preserving the order of `ids`.
    ///
    /// SQLite caps bound parameters per statement, so the ids are read in
    /// chunks that comfortably fit.
    async fn load_prices_by_ids(&self, ids: &[String]) -> Result<Vec<Price>, AppError> {
        let mut by_id = HashMap::with_capacity(ids.len());
        for chunk in ids.chunks(SQLITE_PARAMETER_LIMIT) {
            let sql = format!(
                "SELECT {PRICE_COLUMNS} FROM prices WHERE id IN ({})",
                sql_markers(chunk.len())
            );
            let mut query = sqlx::query_as::<_, PriceRow>(&sql);
            for id in chunk {
                query = query.bind(id);
            }
            let rows = query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
            for row in rows {
                let price = row.into_price()?;
                by_id.insert(price.id.clone(), price);
            }
        }

        ids.iter()
            .map(|id| {
                by_id
                    .get(id)
                    .cloned()
                    .ok_or_else(|| AppError::not_found("price", id))
            })
            .collect()
    }
}
