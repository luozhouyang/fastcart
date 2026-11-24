//! Promotion authoring and rule loading.

//! SQLite-backed promotion service.

use std::collections::HashMap;

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, PageResult, Pagination, PromotionId, now_timestamp};
use sqlx::{QueryBuilder, Sqlite};

use crate::domain::{
    CreatePromotionInput, Promotion, PromotionFilter, PromotionRule, PromotionRuleInput,
    UpdatePromotionInput, normalize_code, validate_per_customer_limit, validate_promotion_fields,
};

/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
use super::*;

async fn load_promotion_rules_tx(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    promotion_id: &str,
) -> Result<Vec<PromotionRule>, AppError> {
    let rows = sqlx::query_as::<_, PromotionRuleRow>(&format!(
        "SELECT {PROMOTION_RULE_COLUMNS} FROM promotion_rules
         WHERE promotion_id = ? ORDER BY created_at, id"
    ))
    .bind(promotion_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)?;
    rows.into_iter().map(PromotionRuleRow::into_rule).collect()
}

pub(super) async fn ensure_promotion_exists_tx(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    id: &str,
) -> Result<(), AppError> {
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM promotions WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?;
    if exists.is_none() {
        return Err(AppError::not_found("promotion", id));
    }
    Ok(())
}

async fn ensure_promotion_code_available_tx(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    code: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM promotions WHERE code = ?")
        .bind(code)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?;
    if let Some(existing) = existing
        && exclude_id != Some(existing.as_str())
    {
        return Err(AppError::conflict("promotion code is already in use"));
    }
    Ok(())
}

async fn insert_promotion_rules(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    promotion_id: &str,
    rules: &[PromotionRuleInput],
    now: i64,
) -> Result<Vec<PromotionRule>, AppError> {
    let mut stored = Vec::with_capacity(rules.len());
    for rule in rules {
        if rule.value.trim().is_empty() {
            return Err(AppError::validation(
                "promotion rule value must not be empty",
            ));
        }
        let id = format!("promrule_{}", PromotionId::new());
        sqlx::query(
            "INSERT INTO promotion_rules
             (id, promotion_id, attribute, operator, value, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(promotion_id)
        .bind(rule.attribute.as_str())
        .bind(rule.operator.as_str())
        .bind(&rule.value)
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        stored.push(PromotionRule {
            id,
            promotion_id: promotion_id.to_owned(),
            attribute: rule.attribute,
            operator: rule.operator,
            value: rule.value.clone(),
            created_at: now,
        });
    }
    Ok(stored)
}

fn push_promotion_filters(builder: &mut QueryBuilder<'_, Sqlite>, filter: &PromotionFilter) {
    if let Some(status) = filter.status {
        builder
            .push(" AND status = ")
            .push_bind(status.as_str().to_owned());
    }
    if let Some(is_automatic) = filter.is_automatic {
        builder
            .push(" AND is_automatic = ")
            .push_bind(if is_automatic { 1_i64 } else { 0_i64 });
    }
    if let Some(promotion_type) = filter.promotion_type {
        builder
            .push(" AND type = ")
            .push_bind(promotion_type.as_str().to_owned());
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PromotionRow {
    id: String,
    code: Option<String>,
    title: String,
    promotion_type: String,
    value: i64,
    status: String,
    is_automatic: i64,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    usage_limit: Option<i64>,
    usage_count: i64,
    per_customer_limit: Option<i64>,
    priority: i64,
    is_stackable: i64,
    target: String,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl PromotionRow {
    fn into_promotion(self) -> Result<Promotion, AppError> {
        Ok(Promotion {
            id: self.id,
            code: self.code,
            title: self.title,
            promotion_type: self.promotion_type.parse()?,
            target: self.target.parse()?,
            value: self.value,
            status: self.status.parse()?,
            is_automatic: self.is_automatic != 0,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            usage_limit: self.usage_limit,
            usage_count: self.usage_count,
            per_customer_limit: self.per_customer_limit,
            priority: self.priority,
            is_stackable: self.is_stackable != 0,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            rules: Vec::new(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PromotionRuleRow {
    id: String,
    promotion_id: String,
    attribute: String,
    operator: String,
    value: String,
    created_at: i64,
}

impl PromotionRuleRow {
    fn into_rule(self) -> Result<PromotionRule, AppError> {
        Ok(PromotionRule {
            id: self.id,
            promotion_id: self.promotion_id,
            attribute: self.attribute.parse()?,
            operator: self.operator.parse()?,
            value: self.value,
            created_at: self.created_at,
        })
    }
}

impl PromotionService {
    /// Creates a promotion and its rules.
    pub async fn create_promotion(
        &self,
        input: CreatePromotionInput,
    ) -> Result<Promotion, AppError> {
        validate_promotion_fields(
            &input.title,
            input.promotion_type,
            input.target,
            input.value,
            input.usage_limit,
            input.starts_at,
            input.ends_at,
            &input.metadata,
        )?;
        validate_per_customer_limit(input.per_customer_limit)?;
        let code = input.code.as_deref().map(normalize_code).transpose()?;

        let id = PromotionId::new().to_string();
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        if let Some(code) = code.as_deref() {
            ensure_promotion_code_available_tx(&mut transaction, code, None).await?;
        }
        sqlx::query(
            "INSERT INTO promotions
             (id, code, title, type, value, status, is_automatic, starts_at, ends_at, usage_limit,
              usage_count, per_customer_limit, priority, is_stackable, target, metadata, created_at,
              updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&code)
        .bind(&input.title)
        .bind(input.promotion_type.as_str())
        .bind(input.value)
        .bind(input.status.as_str())
        .bind(if input.is_automatic { 1_i64 } else { 0_i64 })
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.usage_limit)
        .bind(input.per_customer_limit)
        .bind(input.priority)
        .bind(if input.is_stackable { 1_i64 } else { 0_i64 })
        .bind(input.target.as_str())
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "promotion code is already in use"))?;

        let rules = insert_promotion_rules(&mut transaction, &id, &input.rules, now).await?;
        transaction.commit().await.map_err(database_error)?;

        Ok(Promotion {
            id,
            code,
            title: input.title,
            promotion_type: input.promotion_type,
            target: input.target,
            value: input.value,
            status: input.status,
            is_automatic: input.is_automatic,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            usage_limit: input.usage_limit,
            usage_count: 0,
            per_customer_limit: input.per_customer_limit,
            priority: input.priority,
            is_stackable: input.is_stackable,
            metadata: input.metadata,
            rules,
            created_at: now,
            updated_at: now,
        })
    }

    /// Updates a promotion and optionally replaces its rules.
    pub async fn update_promotion(
        &self,
        id: &str,
        input: UpdatePromotionInput,
    ) -> Result<Promotion, AppError> {
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let existing_row = sqlx::query_as::<_, PromotionRow>(&format!(
            "SELECT {PROMOTION_COLUMNS} FROM promotions WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("promotion", id))?;
        let existing = existing_row.into_promotion()?;
        let created_at = existing.created_at;
        let existing_rules = load_promotion_rules_tx(&mut transaction, id).await?;

        let raw_code = input.code.unwrap_or(existing.code);
        let code = raw_code.as_deref().map(normalize_code).transpose()?;
        let title = input.title.unwrap_or(existing.title);
        let promotion_type = input.promotion_type.unwrap_or(existing.promotion_type);
        let target = input.target.unwrap_or(existing.target);
        let value = input.value.unwrap_or(existing.value);
        let status = input.status.unwrap_or(existing.status);
        let is_automatic = input.is_automatic.unwrap_or(existing.is_automatic);
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        let usage_limit = input.usage_limit.unwrap_or(existing.usage_limit);
        let usage_count = existing.usage_count;
        let per_customer_limit = input
            .per_customer_limit
            .unwrap_or(existing.per_customer_limit);
        let priority = input.priority.unwrap_or(existing.priority);
        let is_stackable = input.is_stackable.unwrap_or(existing.is_stackable);
        let metadata = input.metadata.unwrap_or(existing.metadata);
        validate_per_customer_limit(per_customer_limit)?;

        validate_promotion_fields(
            &title,
            promotion_type,
            target,
            value,
            usage_limit,
            starts_at,
            ends_at,
            &metadata,
        )?;
        if usage_limit.is_some_and(|limit| limit < usage_count) {
            return Err(AppError::validation(
                "usage_limit must not be lower than the recorded usage_count",
            ));
        }
        if let Some(code) = code.as_deref() {
            ensure_promotion_code_available_tx(&mut transaction, code, Some(id)).await?;
        }

        let updated = sqlx::query(
            "UPDATE promotions
             SET code = ?, title = ?, type = ?, value = ?, status = ?, is_automatic = ?,
                 starts_at = ?, ends_at = ?, usage_limit = ?, per_customer_limit = ?,
                 priority = ?, is_stackable = ?, target = ?, metadata = ?, updated_at = ?
             WHERE id = ? AND (? IS NULL OR ? >= usage_count)",
        )
        .bind(&code)
        .bind(&title)
        .bind(promotion_type.as_str())
        .bind(value)
        .bind(status.as_str())
        .bind(if is_automatic { 1_i64 } else { 0_i64 })
        .bind(starts_at)
        .bind(ends_at)
        .bind(usage_limit)
        .bind(per_customer_limit)
        .bind(priority)
        .bind(if is_stackable { 1_i64 } else { 0_i64 })
        .bind(target.as_str())
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .bind(usage_limit)
        .bind(usage_limit)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "promotion code is already in use"))?;
        if updated.rows_affected() == 0 {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM promotions WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(database_error)?;
            if exists == 0 {
                return Err(AppError::not_found("promotion", id));
            }
            return Err(AppError::validation(
                "usage_limit must not be lower than the recorded usage_count",
            ));
        }

        let rules = match input.rules {
            Some(rules) => {
                sqlx::query("DELETE FROM promotion_rules WHERE promotion_id = ?")
                    .bind(id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(database_error)?;
                insert_promotion_rules(&mut transaction, id, &rules, now).await?
            }
            None => existing_rules,
        };
        transaction.commit().await.map_err(database_error)?;

        Ok(Promotion {
            id: id.to_owned(),
            code,
            title,
            promotion_type,
            target,
            value,
            status,
            is_automatic,
            starts_at,
            ends_at,
            usage_limit,
            usage_count,
            per_customer_limit,
            priority,
            is_stackable,
            metadata,
            rules,
            created_at,
            updated_at: now,
        })
    }

    /// Deletes a promotion. Rules and coupons cascade.
    pub async fn delete_promotion(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM promotions WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("promotion", id));
        }
        Ok(())
    }

    /// Loads one promotion with its rules.
    pub async fn get_promotion(&self, id: &str) -> Result<Promotion, AppError> {
        let row = sqlx::query_as::<_, PromotionRow>(&format!(
            "SELECT {PROMOTION_COLUMNS} FROM promotions WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("promotion", id))?;

        let mut promotion = row.into_promotion()?;
        promotion.rules = self.load_promotion_rules(&promotion.id).await?;
        Ok(promotion)
    }

    /// Lists promotions with optional status, automatic and type filters.
    pub async fn list_promotions(
        &self,
        filter: PromotionFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Promotion>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;

        let mut count_builder: QueryBuilder<'_, Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM promotions WHERE 1 = 1");
        push_promotion_filters(&mut count_builder, &filter);
        let count = count_builder
            .build_query_scalar::<i64>()
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(format!(
            "SELECT {PROMOTION_COLUMNS} FROM promotions WHERE 1 = 1"
        ));
        push_promotion_filters(&mut builder, &filter);
        builder
            .push(" ORDER BY created_at, id LIMIT ")
            .push_bind(pagination.limit)
            .push(" OFFSET ")
            .push_bind(pagination.offset);
        let rows = builder
            .build_query_as::<PromotionRow>()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut promotions = rows
            .into_iter()
            .map(PromotionRow::into_promotion)
            .collect::<Result<Vec<_>, AppError>>()?;
        self.attach_promotion_rules(&mut promotions).await?;
        Ok(PageResult::new(promotions, count, pagination))
    }

    /// Lists active automatic promotions valid at `at`.
    pub async fn list_automatic_promotions(&self, at: i64) -> Result<Vec<Promotion>, AppError> {
        let rows = sqlx::query_as::<_, PromotionRow>(&format!(
            "SELECT {PROMOTION_COLUMNS} FROM promotions
             WHERE is_automatic = 1 AND status = 'active'
               AND (starts_at IS NULL OR starts_at <= ?)
               AND (ends_at IS NULL OR ends_at > ?)
               AND (usage_limit IS NULL OR usage_count < usage_limit)
             ORDER BY created_at, id"
        ))
        .bind(at)
        .bind(at)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;

        let mut promotions = rows
            .into_iter()
            .map(PromotionRow::into_promotion)
            .collect::<Result<Vec<_>, AppError>>()?;
        self.attach_promotion_rules(&mut promotions).await?;
        Ok(promotions)
    }

    /// Loads the rules of several promotions in one pass, grouped by
    /// promotion id.
    ///
    /// SQLite caps bound parameters per statement, so the ids are read in
    /// chunks that comfortably fit. Rules keep the `created_at, id` order of the
    /// single-promotion load, and a promotion without rules simply has no entry.
    async fn load_promotion_rules_by_id(
        &self,
        promotion_ids: &[String],
    ) -> Result<HashMap<String, Vec<PromotionRule>>, AppError> {
        let mut by_promotion: HashMap<String, Vec<PromotionRule>> = HashMap::new();
        for chunk in promotion_ids.chunks(SQLITE_PARAMETER_LIMIT) {
            let sql = format!(
                "SELECT {PROMOTION_RULE_COLUMNS} FROM promotion_rules
                 WHERE promotion_id IN ({}) ORDER BY promotion_id, created_at, id",
                sql_markers(chunk.len())
            );
            let mut query = sqlx::query_as::<_, PromotionRuleRow>(&sql);
            for promotion_id in chunk {
                query = query.bind(promotion_id);
            }
            #[cfg(test)]
            self.rule_reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let rows = query
                .fetch_all(self.db.pool())
                .await
                .map_err(database_error)?;
            for row in rows {
                let rule = row.into_rule()?;
                by_promotion
                    .entry(rule.promotion_id.clone())
                    .or_default()
                    .push(rule);
            }
        }
        Ok(by_promotion)
    }

    /// Attaches the rules of every listed promotion in constant query count.
    async fn attach_promotion_rules(&self, promotions: &mut [Promotion]) -> Result<(), AppError> {
        let ids = promotions
            .iter()
            .map(|promotion| promotion.id.clone())
            .collect::<Vec<_>>();
        let mut by_promotion = self.load_promotion_rules_by_id(&ids).await?;
        for promotion in promotions {
            promotion.rules = by_promotion.remove(&promotion.id).unwrap_or_default();
        }
        Ok(())
    }

    async fn load_promotion_rules(
        &self,
        promotion_id: &str,
    ) -> Result<Vec<PromotionRule>, AppError> {
        let ids = [promotion_id.to_owned()];
        let mut by_promotion = self.load_promotion_rules_by_id(&ids).await?;
        Ok(by_promotion.remove(promotion_id).unwrap_or_default())
    }
}
