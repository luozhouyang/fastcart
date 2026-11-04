//! SQLite-backed pricing service.

use std::collections::HashMap;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, PageResult, Pagination, PriceId, PriceListId, PriceSetId, now_timestamp,
};
use sqlx::Row;
use sqlx::{QueryBuilder, Sqlite};

use crate::domain::{
    CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter, PriceList,
    PriceListRule, PriceListRuleInput, PriceListType, PriceSet, SelectedPrice, UpdatePriceInput,
    UpdatePriceListInput, normalize_rule_input, select_price, validate_price_fields,
    validate_price_list_fields,
};

const PRICE_LIST_COLUMNS: &str = "id, title, description, type AS price_type, status, starts_at, \
     ends_at, metadata, created_at, updated_at";
const PRICE_COLUMNS: &str = "id, price_list_id, \
     (SELECT type FROM price_lists WHERE id = prices.price_list_id) AS price_list_type, \
     variant_id, currency_code, amount, min_quantity, max_quantity, customer_group_id, \
     sales_channel_id, starts_at, ends_at, metadata, created_at, updated_at";
const PRICE_LIST_RULE_COLUMNS: &str = "id, price_list_id, attribute, operator, value, created_at";

/// Application service for price lists, prices and deterministic selection.
#[derive(Clone, Debug)]
pub struct PricingService {
    db: StoreDatabase,
}

impl PricingService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db }
    }

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

    /// Updates a price list and optionally replaces its rules.
    pub async fn update_price_list(
        &self,
        id: &str,
        input: UpdatePriceListInput,
    ) -> Result<PriceList, AppError> {
        let existing = self.get_price_list(id).await?;
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
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
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
        .map_err(database_error)?;

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

    // ---------------------------------------------------------------------
    // Price sets
    // ---------------------------------------------------------------------

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

    /// Creates a price.
    pub async fn create_price(&self, input: CreatePriceInput) -> Result<Price, AppError> {
        validate_price_fields(
            &input.variant_id,
            input.amount,
            input.min_quantity,
            input.max_quantity,
            input.starts_at,
            input.ends_at,
            &input.metadata,
        )?;
        let price_list_type = match input.price_list_id.as_deref() {
            Some(price_list_id) => Some(self.ensure_price_list_exists(price_list_id).await?),
            None => None,
        };
        if self
            .find_price_id_by_scope(
                &input.variant_id,
                &input.currency_code,
                input.price_list_id.as_deref(),
                input.customer_group_id.as_deref(),
                input.sales_channel_id.as_deref(),
                input.min_quantity,
            )
            .await?
            .is_some()
        {
            return Err(AppError::conflict(
                "a price already exists for the same scope",
            ));
        }

        let id = PriceId::new().to_string();
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
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
        .bind(&input.customer_group_id)
        .bind(&input.sales_channel_id)
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
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
            customer_group_id: input.customer_group_id,
            sales_channel_id: input.sales_channel_id,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
            price_list_rule_count: 0,
        })
    }

    /// Updates a price.
    pub async fn update_price(&self, id: &str, input: UpdatePriceInput) -> Result<Price, AppError> {
        let existing = self.get_price(id).await?;

        let price_list_id = input.price_list_id.unwrap_or(existing.price_list_id);
        let variant_id = input.variant_id.unwrap_or(existing.variant_id);
        let currency_code = input.currency_code.unwrap_or(existing.currency_code);
        let amount = input.amount.unwrap_or(existing.amount);
        let min_quantity = input.min_quantity.unwrap_or(existing.min_quantity);
        let max_quantity = input.max_quantity.unwrap_or(existing.max_quantity);
        let customer_group_id = input
            .customer_group_id
            .unwrap_or(existing.customer_group_id);
        let sales_channel_id = input.sales_channel_id.unwrap_or(existing.sales_channel_id);
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
            Some(price_list_id) => Some(self.ensure_price_list_exists(price_list_id).await?),
            None => None,
        };
        if let Some(found_id) = self
            .find_price_id_by_scope(
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
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let price_set = ensure_price_set_tx(&mut transaction, &variant_id).await?;
        sqlx::query(
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
        .map_err(database_error)?;
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
            created_at: existing.created_at,
            updated_at: now,
            price_list_rule_count: existing.price_list_rule_count,
        })
    }

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

    /// Inserts or updates a batch of prices keyed by their unique scope.
    pub async fn bulk_upsert_prices(
        &self,
        inputs: Vec<CreatePriceInput>,
    ) -> Result<Vec<Price>, AppError> {
        for input in &inputs {
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
                let exists: Option<String> =
                    sqlx::query_scalar("SELECT id FROM price_lists WHERE id = ?")
                        .bind(price_list_id)
                        .fetch_optional(&mut *transaction)
                        .await
                        .map_err(database_error)?;
                if exists.is_none() {
                    return Err(AppError::not_found("price list", price_list_id));
                }
            }
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
                .map_err(database_error)?;
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
                .map_err(database_error)?;
                id
            };
            ids.push(id);
        }
        transaction.commit().await.map_err(database_error)?;

        let mut prices = Vec::with_capacity(ids.len());
        for id in ids {
            prices.push(self.get_price(&id).await?);
        }
        Ok(prices)
    }

    /// Selects the best eligible price for one variant.
    pub async fn calculate_price(
        &self,
        variant_id: &str,
        context: &PriceContext,
    ) -> Result<Option<SelectedPrice>, AppError> {
        let variant_ids = [variant_id.to_owned()];
        let prices = self.load_eligible_prices(&variant_ids, context).await?;
        Ok(select_price(&prices, context))
    }

    /// Selects the best eligible price for each requested variant.
    pub async fn calculate_prices(
        &self,
        variant_ids: &[String],
        context: &PriceContext,
    ) -> Result<HashMap<String, Option<SelectedPrice>>, AppError> {
        let mut grouped: HashMap<String, Vec<Price>> = HashMap::new();
        for price in self.load_eligible_prices(variant_ids, context).await? {
            grouped
                .entry(price.variant_id.clone())
                .or_default()
                .push(price);
        }

        let empty = Vec::new();
        let mut selected = HashMap::with_capacity(variant_ids.len());
        for variant_id in variant_ids {
            let candidates = grouped.get(variant_id).unwrap_or(&empty);
            selected.insert(variant_id.clone(), select_price(candidates, context));
        }
        Ok(selected)
    }

    async fn load_eligible_prices(
        &self,
        variant_ids: &[String],
        context: &PriceContext,
    ) -> Result<Vec<Price>, AppError> {
        if variant_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
            "SELECT p.id, p.price_list_id, pl.type AS price_list_type, p.variant_id,
                    p.currency_code, p.amount, p.min_quantity, p.max_quantity,
                    p.customer_group_id, p.sales_channel_id, p.starts_at, p.ends_at, p.metadata,
                    p.created_at, p.updated_at
             FROM prices p
             LEFT JOIN price_lists pl ON pl.id = p.price_list_id
             WHERE p.currency_code = ",
        );
        builder.push_bind(context.currency.as_str().to_owned());
        builder.push(" AND p.variant_id IN (");
        let mut separated = builder.separated(", ");
        for variant_id in variant_ids {
            separated.push_bind(variant_id.clone());
        }
        separated.push_unseparated(")");
        builder
            .push(" AND (p.price_list_id IS NULL OR (pl.status = 'active'")
            .push(" AND (pl.starts_at IS NULL OR pl.starts_at <= ")
            .push_bind(context.at)
            .push(") AND (pl.ends_at IS NULL OR pl.ends_at > ")
            .push_bind(context.at)
            .push(")))");
        builder
            .push(" AND (p.starts_at IS NULL OR p.starts_at <= ")
            .push_bind(context.at)
            .push(") AND (p.ends_at IS NULL OR p.ends_at > ")
            .push_bind(context.at)
            .push(")");
        builder
            .push(" AND (p.customer_group_id IS NULL OR p.customer_group_id = ")
            .push_bind(context.customer_group_id.clone())
            .push(") AND (p.sales_channel_id IS NULL OR p.sales_channel_id = ")
            .push_bind(context.sales_channel_id.clone())
            .push(")");
        builder
            .push(" AND p.min_quantity <= ")
            .push_bind(context.quantity)
            .push(" AND (p.max_quantity IS NULL OR p.max_quantity >= ")
            .push_bind(context.quantity)
            .push(")");

        let rows = builder
            .build_query_as::<PriceRow>()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut prices = rows
            .into_iter()
            .map(PriceRow::into_price)
            .collect::<Result<Vec<_>, AppError>>()?;
        let list_ids: Vec<String> = prices
            .iter()
            .filter_map(|price| price.price_list_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let rules = self.load_price_list_rule_conditions(&list_ids).await?;
        prices.retain(|price| {
            price
                .price_list_id
                .as_ref()
                .is_none_or(|list_id| rules_match(rules.get(list_id), context))
        });
        for price in &mut prices {
            price.price_list_rule_count = price
                .price_list_id
                .as_deref()
                .and_then(|list_id| rules.get(list_id))
                .map_or(0, |list_rules| {
                    i64::try_from(list_rules.len()).unwrap_or(i64::MAX)
                });
        }
        Ok(prices)
    }

    async fn load_price_list_rule_conditions(
        &self,
        list_ids: &[String],
    ) -> Result<HashMap<String, Vec<(String, String, String)>>, AppError> {
        if list_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(
            "SELECT price_list_id, attribute, operator, value FROM price_list_rules \
             WHERE price_list_id IN (",
        );
        let mut separated = builder.separated(", ");
        for id in list_ids {
            separated.push_bind(id.clone());
        }
        separated.push_unseparated(")");
        let rows = builder
            .build()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;
        let mut rules: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for row in rows {
            let list_id: String = row.try_get("price_list_id").map_err(database_error)?;
            let attribute: String = row.try_get("attribute").map_err(database_error)?;
            let operator: String = row.try_get("operator").map_err(database_error)?;
            let value: String = row.try_get("value").map_err(database_error)?;
            rules
                .entry(list_id)
                .or_default()
                .push((attribute, operator, value));
        }
        Ok(rules)
    }

    async fn ensure_price_list_exists(&self, id: &str) -> Result<PriceListType, AppError> {
        let price_type: Option<String> =
            sqlx::query_scalar("SELECT type FROM price_lists WHERE id = ?")
                .bind(id)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        match price_type {
            Some(price_type) => price_type.parse(),
            None => Err(AppError::not_found("price list", id)),
        }
    }

    async fn find_price_id_by_scope(
        &self,
        variant_id: &str,
        currency_code: &CurrencyCode,
        price_list_id: Option<&str>,
        customer_group_id: Option<&str>,
        sales_channel_id: Option<&str>,
        min_quantity: i64,
    ) -> Result<Option<String>, AppError> {
        sqlx::query_scalar(
            "SELECT id FROM prices
             WHERE variant_id = ? AND currency_code = ?
               AND IFNULL(price_list_id, '') = ? AND IFNULL(customer_group_id, '') = ?
               AND IFNULL(sales_channel_id, '') = ? AND min_quantity = ?
             LIMIT 1",
        )
        .bind(variant_id)
        .bind(currency_code.as_str())
        .bind(price_list_id.unwrap_or(""))
        .bind(customer_group_id.unwrap_or(""))
        .bind(sales_channel_id.unwrap_or(""))
        .bind(min_quantity)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)
    }

    async fn load_price_list_rules(
        &self,
        price_list_id: &str,
    ) -> Result<Vec<PriceListRule>, AppError> {
        let rows = sqlx::query_as::<_, PriceListRuleRow>(&format!(
            "SELECT {PRICE_LIST_RULE_COLUMNS} FROM price_list_rules
             WHERE price_list_id = ? ORDER BY created_at, id"
        ))
        .bind(price_list_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(rows.into_iter().map(PriceListRuleRow::into_rule).collect())
    }
}

fn push_price_filters(builder: &mut QueryBuilder<'_, Sqlite>, filter: &PriceFilter) {
    if let Some(variant_id) = filter.variant_id.as_deref() {
        builder
            .push(" AND variant_id = ")
            .push_bind(variant_id.to_owned());
    }
    if let Some(price_list_id) = filter.price_list_id.as_deref() {
        builder
            .push(" AND price_list_id = ")
            .push_bind(price_list_id.to_owned());
    }
    if let Some(currency) = filter.currency_code.as_ref() {
        builder
            .push(" AND currency_code = ")
            .push_bind(currency.as_str().to_owned());
    }
}

fn scope_key(
    variant_id: &str,
    currency_code: &CurrencyCode,
    price_list_id: Option<&str>,
    customer_group_id: Option<&str>,
    sales_channel_id: Option<&str>,
    min_quantity: i64,
) -> String {
    format!(
        "{variant_id}|{}|{}|{}|{}|{min_quantity}",
        currency_code.as_str(),
        price_list_id.unwrap_or(""),
        customer_group_id.unwrap_or(""),
        sales_channel_id.unwrap_or("")
    )
}

async fn insert_price_list_rules(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    price_list_id: &str,
    rules: &[PriceListRuleInput],
    now: i64,
) -> Result<Vec<PriceListRule>, AppError> {
    let mut stored = Vec::with_capacity(rules.len());
    for rule in rules {
        let (attribute, operator, value) = normalize_rule_input(rule)?;
        let id = format!("plrule_{}", PriceListId::new());
        sqlx::query(
            "INSERT INTO price_list_rules
             (id, price_list_id, attribute, operator, value, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(price_list_id)
        .bind(&attribute)
        .bind(&operator)
        .bind(&value)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        stored.push(PriceListRule {
            id,
            price_list_id: price_list_id.to_owned(),
            attribute,
            operator,
            value,
            created_at: now,
        });
    }
    Ok(stored)
}

#[derive(Debug, sqlx::FromRow)]
struct PriceListRow {
    id: String,
    title: String,
    description: Option<String>,
    price_type: String,
    status: String,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl PriceListRow {
    fn into_price_list(self) -> Result<PriceList, AppError> {
        Ok(PriceList {
            id: self.id,
            title: self.title,
            description: self.description,
            price_type: self.price_type.parse()?,
            status: self.status.parse()?,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            rules: Vec::new(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PriceListRuleRow {
    id: String,
    price_list_id: String,
    attribute: String,
    operator: String,
    value: String,
    created_at: i64,
}

impl PriceListRuleRow {
    fn into_rule(self) -> PriceListRule {
        PriceListRule {
            id: self.id,
            price_list_id: self.price_list_id,
            attribute: self.attribute,
            operator: self.operator,
            value: self.value,
            created_at: self.created_at,
        }
    }
}

async fn ensure_price_set_tx(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    variant_id: &str,
) -> Result<PriceSet, AppError> {
    if let Some(row) = sqlx::query_as::<_, PriceSetRow>(
        "SELECT ps.id, ps.title, ps.metadata, ps.created_at, ps.updated_at
         FROM price_sets ps
         JOIN product_variant_price_set link ON link.price_set_id = ps.id
         WHERE link.variant_id = ?",
    )
    .bind(variant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    {
        return row.into_price_set();
    }

    let id = PriceSetId::new().to_string();
    let title = format!("Prices for {variant_id}");
    let now = now_timestamp();
    sqlx::query(
        "INSERT INTO price_sets (id, title, metadata, created_at, updated_at)
         VALUES (?, ?, '{}', ?, ?)",
    )
    .bind(&id)
    .bind(&title)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("INSERT INTO product_variant_price_set (variant_id, price_set_id) VALUES (?, ?)")
        .bind(variant_id)
        .bind(&id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(PriceSet {
        id,
        title: Some(title),
        metadata: fastcart_kernel::Metadata::empty(),
        created_at: now,
        updated_at: now,
    })
}

#[derive(Debug, sqlx::FromRow)]
struct PriceSetRow {
    id: String,
    title: Option<String>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl PriceSetRow {
    fn into_price_set(self) -> Result<PriceSet, AppError> {
        Ok(PriceSet {
            id: self.id,
            title: self.title,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PriceRow {
    id: String,
    price_list_id: Option<String>,
    price_list_type: Option<String>,
    variant_id: String,
    currency_code: String,
    amount: i64,
    min_quantity: i64,
    max_quantity: Option<i64>,
    customer_group_id: Option<String>,
    sales_channel_id: Option<String>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl PriceRow {
    fn into_price(self) -> Result<Price, AppError> {
        Ok(Price {
            id: self.id,
            price_list_id: self.price_list_id,
            price_list_type: self
                .price_list_type
                .map(|price_type| price_type.parse())
                .transpose()?,
            variant_id: self.variant_id,
            currency_code: CurrencyCode::new(self.currency_code)?,
            amount: self.amount,
            min_quantity: self.min_quantity,
            max_quantity: self.max_quantity,
            customer_group_id: self.customer_group_id,
            sales_channel_id: self.sales_channel_id,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
            price_list_rule_count: 0,
        })
    }
}

fn rules_match(rules: Option<&Vec<(String, String, String)>>, context: &PriceContext) -> bool {
    rules.is_none_or(|rules| {
        rules.iter().all(|(attribute, operator, value)| {
            let actual = match attribute.as_str() {
                "customer_group_id" => context.customer_group_id.as_deref(),
                "sales_channel_id" => context.sales_channel_id.as_deref(),
                "currency" | "currency_code" => Some(context.currency.as_str()),
                _ => None,
            };
            match operator.as_str() {
                "eq" => actual == Some(value.as_str()),
                "ne" => actual != Some(value.as_str()),
                _ => false,
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use fastcart_infrastructure::StoreDatabase;
    use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

    use super::PricingService;
    use crate::domain::{
        CreatePriceInput, CreatePriceListInput, Price, PriceContext, PriceFilter,
        PriceListRuleInput, PriceListStatus, PriceListType, UpdatePriceInput, UpdatePriceListInput,
        select_price,
    };

    async fn service() -> PricingService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        PricingService::new(db)
    }

    fn usd() -> CurrencyCode {
        CurrencyCode::new("USD").unwrap()
    }

    fn eur() -> CurrencyCode {
        CurrencyCode::new("EUR").unwrap()
    }

    fn context(quantity: i64, at: i64) -> PriceContext {
        PriceContext {
            currency: usd(),
            quantity,
            customer_group_id: None,
            sales_channel_id: None,
            at,
        }
    }

    fn sample_price(id: &str, amount: i64) -> Price {
        Price {
            id: id.to_owned(),
            price_list_id: None,
            price_list_type: None,
            variant_id: "var_1".to_owned(),
            currency_code: usd(),
            amount,
            min_quantity: 1,
            max_quantity: None,
            customer_group_id: None,
            sales_channel_id: None,
            starts_at: None,
            ends_at: None,
            metadata: Metadata::empty(),
            created_at: 0,
            updated_at: 0,
            price_list_rule_count: 0,
        }
    }

    fn sale_price(id: &str, list_id: &str, amount: i64) -> Price {
        let mut price = sample_price(id, amount);
        price.price_list_id = Some(list_id.to_owned());
        price.price_list_type = Some(PriceListType::Sale);
        price
    }

    fn override_price(id: &str, list_id: &str, amount: i64) -> Price {
        let mut price = sample_price(id, amount);
        price.price_list_id = Some(list_id.to_owned());
        price.price_list_type = Some(PriceListType::Override);
        price
    }

    #[test]
    fn selector_filters_by_currency() {
        let usd_price = sample_price("price_usd", 1000);
        let mut eur_price = sample_price("price_eur", 900);
        eur_price.currency_code = eur();

        let selected = select_price(&[eur_price, usd_price], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_usd");
        assert_eq!(selected.amount, 1000);
        assert!(selected.price_list_id.is_none());
    }

    #[test]
    fn selector_uses_quantity_tiers() {
        let base = sample_price("price_base", 1000);
        let mut tier = sample_price("price_tier", 900);
        tier.min_quantity = 10;

        let low = select_price(&[base.clone(), tier.clone()], &context(5, 100)).unwrap();
        assert_eq!(low.price_id, "price_base");
        let high = select_price(&[base, tier], &context(10, 100)).unwrap();
        assert_eq!(high.price_id, "price_tier");
    }

    #[test]
    fn selector_respects_quantity_upper_bounds() {
        let base = sample_price("price_base", 1000);
        let mut tier = sample_price("price_tier", 900);
        tier.min_quantity = 10;
        tier.max_quantity = Some(20);

        assert_eq!(
            select_price(&[base.clone(), tier.clone()], &context(21, 100))
                .unwrap()
                .price_id,
            "price_base"
        );
        assert_eq!(
            select_price(&[base, tier], &context(20, 100))
                .unwrap()
                .price_id,
            "price_tier"
        );
    }

    #[test]
    fn selector_ignores_expired_prices() {
        let mut stale = sample_price("price_stale", 100);
        stale.ends_at = Some(100);
        let mut current = sample_price("price_current", 200);
        current.starts_at = Some(50);
        current.ends_at = Some(200);

        let selected = select_price(&[stale, current], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_current");
    }

    #[test]
    fn selector_prefers_customer_group_scope() {
        let base = sample_price("price_base", 1000);
        let mut scoped = sample_price("price_group", 900);
        scoped.customer_group_id = Some("vip".to_owned());

        let mut scoped_context = context(1, 100);
        scoped_context.customer_group_id = Some("vip".to_owned());
        assert_eq!(
            select_price(&[base.clone(), scoped.clone()], &scoped_context)
                .unwrap()
                .price_id,
            "price_group"
        );

        assert_eq!(
            select_price(&[base, scoped], &context(1, 100))
                .unwrap()
                .price_id,
            "price_base"
        );
    }

    #[test]
    fn selector_prefers_sales_channel_scope() {
        let base = sample_price("price_base", 1000);
        let mut scoped = sample_price("price_channel", 900);
        scoped.sales_channel_id = Some("web".to_owned());

        let mut channel_context = context(1, 100);
        channel_context.sales_channel_id = Some("web".to_owned());
        assert_eq!(
            select_price(&[base, scoped], &channel_context)
                .unwrap()
                .price_id,
            "price_channel"
        );
    }

    #[test]
    fn selector_applies_sale_only_when_it_is_a_discount() {
        let base = sample_price("price_base", 1000);
        let cheaper = sale_price("price_sale", "plist_1", 900);
        let selected = select_price(&[base.clone(), cheaper], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_sale");
        assert_eq!(selected.price_list_id.as_deref(), Some("plist_1"));
        assert_eq!(selected.price_list_type, Some(PriceListType::Sale));
        assert_eq!(selected.original_amount, Some(1000));

        let expensive = sale_price("price_sale", "plist_1", 1100);
        let selected = select_price(&[base, expensive], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_base");
        assert_eq!(selected.price_list_type, None);
        assert_eq!(selected.original_amount, None);
    }

    #[test]
    fn selector_uses_sale_when_no_base_price_exists() {
        let only_sale = sale_price("price_sale", "plist_1", 900);
        let selected = select_price(&[only_sale], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_sale");
        assert_eq!(selected.original_amount, None);
    }

    #[test]
    fn selector_override_replaces_base_and_sale() {
        let base = sample_price("price_base", 1000);
        let sale = sale_price("price_sale", "plist_sale", 700);
        let over = override_price("price_override", "plist_override", 1200);

        let selected = select_price(&[base, sale, over], &context(1, 100)).unwrap();
        assert_eq!(selected.price_id, "price_override");
        assert_eq!(selected.price_list_type, Some(PriceListType::Override));
        assert_eq!(selected.original_amount, Some(1000));
    }

    #[test]
    fn selector_ranks_multiple_matching_lists_deterministically() {
        let base = sample_price("price_base", 1000);

        // More matching rules beat a cheaper but less specific list.
        let mut generic = sale_price("price_generic", "plist_generic", 800);
        generic.price_list_rule_count = 0;
        let mut specific = sale_price("price_specific", "plist_specific", 950);
        specific.price_list_rule_count = 1;
        assert_eq!(
            select_price(
                &[base.clone(), generic.clone(), specific.clone()],
                &context(1, 100)
            )
            .unwrap()
            .price_id,
            "price_specific"
        );

        // With equal specificity the lowest amount wins.
        let mut cheaper = generic.clone();
        cheaper.price_list_rule_count = 1;
        cheaper.amount = 700;
        assert_eq!(
            select_price(&[base.clone(), specific, cheaper], &context(1, 100))
                .unwrap()
                .price_id,
            "price_generic"
        );

        // With equal specificity and amount the newest wins.
        let mut newest = generic;
        newest.price_list_rule_count = 1;
        newest.amount = 700;
        newest.starts_at = Some(50);
        let older = {
            let mut older = newest.clone();
            older.id = "price_older".to_owned();
            older.starts_at = Some(10);
            older
        };
        assert_eq!(
            select_price(&[base, older, newest], &context(1, 100))
                .unwrap()
                .price_id,
            "price_generic"
        );
    }

    #[test]
    fn selector_tie_breaks_are_deterministic() {
        let cheap = sample_price("price_cheap", 1000);
        let expensive = sample_price("price_expensive", 1100);
        assert_eq!(
            select_price(&[expensive, cheap], &context(1, 100))
                .unwrap()
                .price_id,
            "price_cheap"
        );

        let alpha = sample_price("price_a", 1000);
        let beta = sample_price("price_b", 1000);
        assert_eq!(
            select_price(&[beta, alpha], &context(1, 100))
                .unwrap()
                .price_id,
            "price_a"
        );
    }

    #[test]
    fn selector_returns_none_without_a_match() {
        assert!(select_price(&[], &context(1, 100)).is_none());
        let mut price = sample_price("price_future", 1000);
        price.starts_at = Some(200);
        assert!(select_price(&[price], &context(1, 100)).is_none());
    }

    #[tokio::test]
    async fn creates_and_reads_price_lists_with_rules() {
        let service = service().await;
        let input = CreatePriceListInput::new("Summer sale")
            .active()
            .with_rule(PriceListRuleInput::eq("customer_group_id", "\"vip\""));

        let created = service.create_price_list(input).await.unwrap();
        assert_eq!(created.title, "Summer sale");
        assert_eq!(created.status, PriceListStatus::Active);
        assert_eq!(created.rules.len(), 1);

        let fetched = service.get_price_list(&created.id).await.unwrap();
        assert_eq!(fetched, created);
    }

    #[tokio::test]
    async fn updates_price_lists_and_replaces_rules() {
        let service = service().await;
        let created = service
            .create_price_list(CreatePriceListInput::new("Sale").active())
            .await
            .unwrap();

        let updated = service
            .update_price_list(
                &created.id,
                UpdatePriceListInput {
                    title: Some("Autumn sale".to_owned()),
                    price_type: Some(PriceListType::Override),
                    rules: Some(vec![PriceListRuleInput::eq("sales_channel_id", "\"web\"")]),
                    ..UpdatePriceListInput::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.title, "Autumn sale");
        assert_eq!(updated.price_type, PriceListType::Override);
        assert_eq!(updated.rules.len(), 1);
        assert_eq!(updated.created_at, created.created_at);
    }

    #[tokio::test]
    async fn lists_price_lists_with_pagination() {
        let service = service().await;
        for index in 0..3 {
            service
                .create_price_list(CreatePriceListInput::new(format!("Sale {index}")))
                .await
                .unwrap();
        }

        let page = service
            .list_price_lists(Pagination {
                limit: 2,
                offset: 0,
            })
            .await
            .unwrap();
        assert_eq!(page.count, 3);
        assert_eq!(page.data.len(), 2);
    }

    #[tokio::test]
    async fn rejects_duplicate_price_scope() {
        let service = service().await;
        let input = CreatePriceInput::new("var_1", usd(), 1000);
        service.create_price(input.clone()).await.unwrap();

        let error = service.create_price(input).await.unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn validates_price_inputs() {
        let service = service().await;
        let negative = CreatePriceInput::new("var_1", usd(), -1);
        assert!(matches!(
            service.create_price(negative).await.unwrap_err(),
            AppError::Validation(_)
        ));

        let mut zero_minimum = CreatePriceInput::new("var_1", usd(), 1000);
        zero_minimum.min_quantity = 0;
        assert!(matches!(
            service.create_price(zero_minimum).await.unwrap_err(),
            AppError::Validation(_)
        ));

        let mut inverted_range = CreatePriceInput::new("var_1", usd(), 1000);
        inverted_range.min_quantity = 3;
        inverted_range.max_quantity = Some(2);
        assert!(matches!(
            service.create_price(inverted_range).await.unwrap_err(),
            AppError::Validation(_)
        ));

        let missing_list =
            CreatePriceInput::new("var_1", usd(), 1000).with_price_list("plist_missing");
        assert!(matches!(
            service.create_price(missing_list).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn manages_prices_with_filters_updates_and_deletes() {
        let service = service().await;
        let first = service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_2", usd(), 2000))
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", eur(), 900))
            .await
            .unwrap();

        let by_variant = service
            .list_prices(
                PriceFilter {
                    variant_id: Some("var_1".to_owned()),
                    ..PriceFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(by_variant.count, 2);

        let by_currency = service
            .list_prices(
                PriceFilter {
                    currency_code: Some(eur()),
                    ..PriceFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(by_currency.count, 1);

        let updated = service
            .update_price(
                &first.id,
                UpdatePriceInput {
                    amount: Some(750),
                    max_quantity: Some(Some(5)),
                    ..UpdatePriceInput::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.amount, 750);
        assert_eq!(updated.max_quantity, Some(5));
        assert_eq!(updated.created_at, first.created_at);

        service.delete_price(&first.id).await.unwrap();
        assert!(matches!(
            service.get_price(&first.id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            service.delete_price(&first.id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn bulk_upsert_updates_existing_scope() {
        let service = service().await;
        let initial = vec![
            CreatePriceInput::new("var_1", usd(), 1000),
            CreatePriceInput::new("var_1", usd(), 900).with_quantity_range(10, None),
        ];
        let inserted = service.bulk_upsert_prices(initial).await.unwrap();
        assert_eq!(inserted.len(), 2);

        let updated = service
            .bulk_upsert_prices(vec![
                CreatePriceInput::new("var_1", usd(), 800),
                CreatePriceInput::new("var_1", usd(), 700).with_quantity_range(10, None),
            ])
            .await
            .unwrap();
        assert_eq!(updated.len(), 2);
        assert!(updated.iter().any(|price| price.amount == 800));
        assert!(updated.iter().any(|price| price.amount == 700));

        let all = service
            .list_prices(
                PriceFilter {
                    variant_id: Some("var_1".to_owned()),
                    ..PriceFilter::default()
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(all.count, 2);
    }

    #[tokio::test]
    async fn price_list_rules_restrict_selection() {
        let service = service().await;
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();
        let list = service
            .create_price_list(
                CreatePriceListInput::new("VIP")
                    .active()
                    .with_rule(PriceListRuleInput::eq("customer_group_id", "vip")),
            )
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&list.id))
            .await
            .unwrap();

        assert_eq!(
            service
                .calculate_price("var_1", &context(1, 100))
                .await
                .unwrap()
                .unwrap()
                .amount,
            1000
        );
        let mut vip = context(1, 100);
        vip.customer_group_id = Some("vip".to_owned());
        assert_eq!(
            service
                .calculate_price("var_1", &vip)
                .await
                .unwrap()
                .unwrap()
                .amount,
            800
        );
    }

    #[tokio::test]
    async fn calculation_ignores_inactive_and_expired_price_lists() {
        let service = service().await;
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();

        let draft = service
            .create_price_list(CreatePriceListInput::new("Draft sale"))
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 500).with_price_list(&draft.id))
            .await
            .unwrap();
        assert_eq!(
            service
                .calculate_price("var_1", &context(1, 100))
                .await
                .unwrap()
                .unwrap()
                .amount,
            1000
        );

        let active = service
            .create_price_list(CreatePriceListInput::new("Active sale").active())
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&active.id))
            .await
            .unwrap();
        assert_eq!(
            service
                .calculate_price("var_1", &context(1, 100))
                .await
                .unwrap()
                .unwrap()
                .amount,
            800
        );

        let expired = service
            .create_price_list(
                CreatePriceListInput::new("Expired sale")
                    .active()
                    .with_window(Some(0), Some(50)),
            )
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 100).with_price_list(&expired.id))
            .await
            .unwrap();
        assert_eq!(
            service
                .calculate_price("var_1", &context(1, 100))
                .await
                .unwrap()
                .unwrap()
                .amount,
            800
        );
    }

    #[tokio::test]
    async fn calculation_applies_scopes_and_batch_variants() {
        let service = service().await;
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 900).with_customer_group("vip"))
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_2", usd(), 2000))
            .await
            .unwrap();

        let mut vip_context = context(1, 100);
        vip_context.customer_group_id = Some("vip".to_owned());
        let selected = service
            .calculate_price("var_1", &vip_context)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.amount, 900);
        assert_eq!(
            service
                .calculate_price("var_1", &context(1, 100))
                .await
                .unwrap()
                .unwrap()
                .amount,
            1000
        );

        let batch = service
            .calculate_prices(
                &[
                    "var_1".to_owned(),
                    "var_2".to_owned(),
                    "var_missing".to_owned(),
                ],
                &context(1, 100),
            )
            .await
            .unwrap();
        assert_eq!(batch["var_1"].as_ref().unwrap().amount, 1000);
        assert_eq!(batch["var_2"].as_ref().unwrap().amount, 2000);
        assert!(batch["var_missing"].is_none());
    }

    #[tokio::test]
    async fn price_sets_are_created_once_per_variant_and_linked_to_prices() {
        let service = service().await;
        let base = service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();
        let price_set = service
            .get_price_set_for_variant("var_1")
            .await
            .unwrap()
            .expect("price set");
        assert_eq!(price_set.title.as_deref(), Some("Prices for var_1"));

        let stored_set_id: String =
            sqlx::query_scalar("SELECT price_set_id FROM prices WHERE id = ?")
                .bind(&base.id)
                .fetch_one(service.db.pool())
                .await
                .unwrap();
        assert_eq!(stored_set_id, price_set.id);

        let list = service
            .create_price_list(CreatePriceListInput::new("Sale").active())
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 900).with_price_list(&list.id))
            .await
            .unwrap();
        let same_set = service
            .get_or_create_price_set_for_variant("var_1")
            .await
            .unwrap();
        assert_eq!(same_set.id, price_set.id);

        let other_set = service
            .get_or_create_price_set_for_variant("var_2")
            .await
            .unwrap();
        assert_ne!(other_set.id, price_set.id);
        assert!(
            service
                .get_price_set_for_variant("var_missing")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn calculation_treats_sales_as_discounts_and_overrides_as_replacements() {
        let service = service().await;
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 1000))
            .await
            .unwrap();

        let sale = service
            .create_price_list(CreatePriceListInput::new("Summer sale").active())
            .await
            .unwrap();
        service
            .create_price(CreatePriceInput::new("var_1", usd(), 800).with_price_list(&sale.id))
            .await
            .unwrap();
        let selected = service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.amount, 800);
        assert_eq!(selected.price_list_type, Some(PriceListType::Sale));
        assert_eq!(selected.original_amount, Some(1000));

        let overpriced = service
            .create_price_list(CreatePriceListInput::new("Bad sale").active())
            .await
            .unwrap();
        service
            .create_price(
                CreatePriceInput::new("var_1", usd(), 1200).with_price_list(&overpriced.id),
            )
            .await
            .unwrap();
        let selected = service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.amount, 800);
        assert_eq!(selected.price_list_id.as_deref(), Some(sale.id.as_str()));

        let override_list = service
            .create_price_list({
                let mut input = CreatePriceListInput::new("Absolute").active();
                input.price_type = PriceListType::Override;
                input
            })
            .await
            .unwrap();
        service
            .create_price(
                CreatePriceInput::new("var_1", usd(), 1500).with_price_list(&override_list.id),
            )
            .await
            .unwrap();
        let selected = service
            .calculate_price("var_1", &context(1, 100))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.amount, 1500);
        assert_eq!(selected.price_list_type, Some(PriceListType::Override));
        assert_eq!(selected.original_amount, Some(1000));
    }
}
