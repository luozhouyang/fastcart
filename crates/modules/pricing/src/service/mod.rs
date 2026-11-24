//! SQLite-backed pricing service.

use std::collections::HashMap;

use fastcart_infrastructure::{StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CurrencyCode, PageResult, Pagination, PriceId, PriceListId, PriceSetId, now_timestamp,
};
use sqlx::Row;
use sqlx::{QueryBuilder, Sqlite, Transaction};

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

/// Maximum number of bound parameters used in one `IN (...)` statement.
/// SQLite's default limit is 999; staying under it leaves headroom for the
/// other parameters a query may carry.
const SQLITE_PARAMETER_LIMIT: usize = 900;

mod calculation;
mod price_lists;
mod prices;

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
}

fn sql_markers(count: usize) -> String {
    vec!["?"; count].join(", ")
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            AppError::conflict(conflict_message)
        }
        _ => database_error(error),
    }
}

/// Trims an optional scope identifier and folds blanks to `None`.
fn normalize_scope(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

async fn ensure_scope_ids_exist_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    customer_group_id: Option<&str>,
    sales_channel_id: Option<&str>,
) -> Result<(), AppError> {
    if let Some(customer_group_id) = customer_group_id {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM customer_groups WHERE id = ?")
                .bind(customer_group_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("customer group", customer_group_id));
        }
    }
    if let Some(sales_channel_id) = sales_channel_id {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM sales_channels WHERE id = ?")
                .bind(sales_channel_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(database_error)?;
        if existing.is_none() {
            return Err(AppError::not_found("sales channel", sales_channel_id));
        }
    }
    Ok(())
}

async fn ensure_price_list_exists_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<PriceListType, AppError> {
    let price_type: Option<String> =
        sqlx::query_scalar("SELECT type FROM price_lists WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(database_error)?;
    match price_type {
        Some(price_type) => price_type.parse(),
        None => Err(AppError::not_found("price list", id)),
    }
}

async fn find_price_id_by_scope_tx(
    transaction: &mut Transaction<'_, Sqlite>,
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
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)
}

async fn get_price_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<Price, AppError> {
    let row =
        sqlx::query_as::<_, PriceRow>(&format!("SELECT {PRICE_COLUMNS} FROM prices WHERE id = ?"))
            .bind(id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(database_error)?
            .ok_or_else(|| AppError::not_found("price", id))?;
    row.into_price()
}

async fn load_price_list_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<PriceList, AppError> {
    let row = sqlx::query_as::<_, PriceListRow>(&format!(
        "SELECT {PRICE_LIST_COLUMNS} FROM price_lists WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| AppError::not_found("price list", id))?;
    let mut price_list = row.into_price_list()?;
    price_list.rules = load_price_list_rules_tx(transaction, &price_list.id).await?;
    Ok(price_list)
}

async fn load_price_list_rules_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    price_list_id: &str,
) -> Result<Vec<PriceListRule>, AppError> {
    let rows = sqlx::query_as::<_, PriceListRuleRow>(&format!(
        "SELECT {PRICE_LIST_RULE_COLUMNS} FROM price_list_rules
         WHERE price_list_id = ? ORDER BY created_at, id"
    ))
    .bind(price_list_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(rows.into_iter().map(PriceListRuleRow::into_rule).collect())
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
    fastcart_module_catalog::CatalogService::attach_price_set(transaction, variant_id, &id).await?;
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
mod tests;
