//! Price selection and calculation.

use super::*;

impl PricingService {
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
}

impl PricingService {
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
}

impl PricingService {
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
}

impl PricingService {
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
}

impl PricingService {
    pub(super) async fn load_price_list_rules(
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
