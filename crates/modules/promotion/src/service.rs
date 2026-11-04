//! SQLite-backed promotion service.

use fastcart_infrastructure::{AuditActor, StoreDatabase, database_error};
use fastcart_kernel::{
    AppError, CouponId, EventId, PageResult, Pagination, PromotionId, now_timestamp,
    validate_metadata,
};
use sqlx::{QueryBuilder, Sqlite};

use crate::domain::{
    Campaign, CampaignBudgetType, Coupon, CouponFilter, CreateCampaignInput, CreateCouponInput,
    CreatePromotionInput, Promotion, PromotionCart, PromotionContext, PromotionFilter,
    PromotionResult, PromotionRule, PromotionRuleInput, PromotionStatus, PromotionUsage,
    UpdateCampaignInput, UpdateCouponInput, UpdatePromotionInput, calculate_promotions,
    normalize_code, validate_campaign_budget, validate_promotion_fields, validate_usage_limit,
    validate_window, window_contains,
};

const PROMOTION_COLUMNS: &str = "id, code, title, type AS promotion_type, value, status, \
     is_automatic, starts_at, ends_at, usage_limit, usage_count, metadata, created_at, updated_at";
const PROMOTION_RULE_COLUMNS: &str = "id, promotion_id, attribute, operator, value, created_at";
const COUPON_COLUMNS: &str = "id, code, promotion_id, usage_limit, usage_count, starts_at, \
     ends_at, metadata, created_at, updated_at";

/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
#[derive(Clone, Debug)]
pub struct PromotionService {
    db: StoreDatabase,
    audit: Option<AuditActor>,
}

impl PromotionService {
    /// Creates a service over an already-migrated store database.
    #[must_use]
    pub fn new(db: StoreDatabase) -> Self {
        Self { db, audit: None }
    }

    /// Records campaign mutations in the store audit outbox.
    ///
    /// When an actor is set, every mutating campaign method appends its audit
    /// entry in the same transaction as the change.
    #[must_use]
    pub fn with_audit_actor(mut self, actor_id: Option<String>) -> Self {
        self.audit = actor_id.map(|id| AuditActor::new(id, self.db.store_id().as_str()));
        self
    }

    /// Appends one audit entry to the outbox inside a mutation transaction.
    async fn append_audit(
        &self,
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if let Some(audit) = &self.audit {
            audit
                .append(&mut **tx, action, resource_type, resource_id, data)
                .await
                .map_err(database_error)?;
        }
        Ok(())
    }

    /// Creates a promotion and its rules.
    pub async fn create_promotion(
        &self,
        input: CreatePromotionInput,
    ) -> Result<Promotion, AppError> {
        validate_promotion_fields(
            &input.title,
            input.promotion_type,
            input.value,
            input.usage_limit,
            input.starts_at,
            input.ends_at,
            &input.metadata,
        )?;
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
              usage_count, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
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
            value: input.value,
            status: input.status,
            is_automatic: input.is_automatic,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            usage_limit: input.usage_limit,
            usage_count: 0,
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
        let value = input.value.unwrap_or(existing.value);
        let status = input.status.unwrap_or(existing.status);
        let is_automatic = input.is_automatic.unwrap_or(existing.is_automatic);
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        let usage_limit = input.usage_limit.unwrap_or(existing.usage_limit);
        let usage_count = existing.usage_count;
        let metadata = input.metadata.unwrap_or(existing.metadata);

        validate_promotion_fields(
            &title,
            promotion_type,
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
                 starts_at = ?, ends_at = ?, usage_limit = ?, metadata = ?, updated_at = ?
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
            value,
            status,
            is_automatic,
            starts_at,
            ends_at,
            usage_limit,
            usage_count,
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

        let mut promotions = Vec::with_capacity(rows.len());
        for row in rows {
            let mut promotion = row.into_promotion()?;
            promotion.rules = self.load_promotion_rules(&promotion.id).await?;
            promotions.push(promotion);
        }
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

        let mut promotions = Vec::with_capacity(rows.len());
        for row in rows {
            let mut promotion = row.into_promotion()?;
            promotion.rules = self.load_promotion_rules(&promotion.id).await?;
            promotions.push(promotion);
        }
        Ok(promotions)
    }

    /// Creates a coupon.
    pub async fn create_coupon(&self, input: CreateCouponInput) -> Result<Coupon, AppError> {
        let code = normalize_code(&input.code)?;
        validate_usage_limit(input.usage_limit, "coupon usage_limit")?;
        validate_window(input.starts_at, input.ends_at, "coupon")?;
        validate_metadata(&input.metadata)?;

        let id = CouponId::new().to_string();
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        ensure_promotion_exists_tx(&mut transaction, &input.promotion_id).await?;
        ensure_coupon_code_available_tx(&mut transaction, &code, None).await?;
        sqlx::query(
            "INSERT INTO coupons
             (id, code, promotion_id, usage_limit, usage_count, starts_at, ends_at, metadata,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, 0, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&code)
        .bind(&input.promotion_id)
        .bind(input.usage_limit)
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "coupon code is already in use"))?;
        transaction.commit().await.map_err(database_error)?;

        Ok(Coupon {
            id,
            code,
            promotion_id: input.promotion_id,
            usage_limit: input.usage_limit,
            usage_count: 0,
            starts_at: input.starts_at,
            ends_at: input.ends_at,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        })
    }

    /// Updates a coupon.
    pub async fn update_coupon(
        &self,
        id: &str,
        input: UpdateCouponInput,
    ) -> Result<Coupon, AppError> {
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let existing_row = sqlx::query_as::<_, CouponRow>(&format!(
            "SELECT {COUPON_COLUMNS} FROM coupons WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("coupon", id))?;
        let existing = existing_row.into_coupon();
        let created_at = existing.created_at;

        let code = match input.code {
            Some(code) => normalize_code(&code)?,
            None => existing.code,
        };
        let promotion_id = input.promotion_id.unwrap_or(existing.promotion_id);
        let usage_limit = input.usage_limit.unwrap_or(existing.usage_limit);
        let usage_count = existing.usage_count;
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        let metadata = input.metadata.unwrap_or(existing.metadata);

        validate_usage_limit(usage_limit, "coupon usage_limit")?;
        if usage_limit.is_some_and(|limit| limit < usage_count) {
            return Err(AppError::validation(
                "usage_limit must not be lower than the recorded usage_count",
            ));
        }
        validate_window(starts_at, ends_at, "coupon")?;
        validate_metadata(&metadata)?;
        ensure_promotion_exists_tx(&mut transaction, &promotion_id).await?;
        ensure_coupon_code_available_tx(&mut transaction, &code, Some(id)).await?;

        let updated = sqlx::query(
            "UPDATE coupons
             SET code = ?, promotion_id = ?, usage_limit = ?, starts_at = ?, ends_at = ?,
                 metadata = ?, updated_at = ?
             WHERE id = ? AND (? IS NULL OR ? >= usage_count)",
        )
        .bind(&code)
        .bind(&promotion_id)
        .bind(usage_limit)
        .bind(starts_at)
        .bind(ends_at)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .bind(usage_limit)
        .bind(usage_limit)
        .execute(&mut *transaction)
        .await
        .map_err(|error| unique_or_database(error, "coupon code is already in use"))?;
        if updated.rows_affected() == 0 {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coupons WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await
                .map_err(database_error)?;
            if exists == 0 {
                return Err(AppError::not_found("coupon", id));
            }
            return Err(AppError::validation(
                "usage_limit must not be lower than the recorded usage_count",
            ));
        }
        transaction.commit().await.map_err(database_error)?;

        Ok(Coupon {
            id: id.to_owned(),
            code,
            promotion_id,
            usage_limit,
            usage_count,
            starts_at,
            ends_at,
            metadata,
            created_at,
            updated_at: now,
        })
    }

    /// Deletes a coupon.
    pub async fn delete_coupon(&self, id: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM coupons WHERE id = ?")
            .bind(id)
            .execute(self.db.pool())
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("coupon", id));
        }
        Ok(())
    }

    /// Loads one coupon.
    pub async fn get_coupon(&self, id: &str) -> Result<Coupon, AppError> {
        let row = sqlx::query_as::<_, CouponRow>(&format!(
            "SELECT {COUPON_COLUMNS} FROM coupons WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("coupon", id))?;
        Ok(row.into_coupon())
    }

    /// Lists coupons with an optional promotion filter.
    pub async fn list_coupons(
        &self,
        filter: CouponFilter,
        pagination: Pagination,
    ) -> Result<PageResult<Coupon>, AppError> {
        let pagination = Pagination::new(Some(pagination.limit), Some(pagination.offset))?;

        let mut count_builder: QueryBuilder<'_, Sqlite> =
            QueryBuilder::new("SELECT COUNT(*) FROM coupons WHERE 1 = 1");
        push_coupon_filters(&mut count_builder, &filter);
        let count = count_builder
            .build_query_scalar::<i64>()
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;

        let mut builder: QueryBuilder<'_, Sqlite> =
            QueryBuilder::new(format!("SELECT {COUPON_COLUMNS} FROM coupons WHERE 1 = 1"));
        push_coupon_filters(&mut builder, &filter);
        builder
            .push(" ORDER BY created_at, id LIMIT ")
            .push_bind(pagination.limit)
            .push(" OFFSET ")
            .push_bind(pagination.offset);
        let rows = builder
            .build_query_as::<CouponRow>()
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)?;

        let coupons = rows.into_iter().map(CouponRow::into_coupon).collect();
        Ok(PageResult::new(coupons, count, pagination))
    }

    /// Validates a coupon and its promotion at `at`.
    ///
    /// Returns `Conflict` for inactive, expired or exhausted coupons and
    /// `NotFound` when the coupon or its promotion no longer exists.
    pub async fn validate_coupon(&self, code: &str, at: i64) -> Result<Coupon, AppError> {
        let code = normalize_code(code)?;
        let coupon = self
            .find_coupon_by_code(&code)
            .await?
            .ok_or_else(|| AppError::not_found("coupon", code.clone()))?;

        if !window_contains(at, coupon.starts_at, coupon.ends_at) {
            return Err(AppError::conflict("coupon is outside its validity window"));
        }
        if coupon
            .usage_limit
            .is_some_and(|limit| coupon.usage_count >= limit)
        {
            return Err(AppError::conflict("coupon usage limit reached"));
        }

        let promotion = self.get_promotion(&coupon.promotion_id).await?;
        if promotion.status != PromotionStatus::Active {
            return Err(AppError::conflict("promotion is not active"));
        }
        if !window_contains(at, promotion.starts_at, promotion.ends_at) {
            return Err(AppError::conflict(
                "promotion is outside its validity window",
            ));
        }
        if promotion
            .usage_limit
            .is_some_and(|limit| promotion.usage_count >= limit)
        {
            return Err(AppError::conflict("promotion usage limit reached"));
        }
        Ok(coupon)
    }

    /// Creates a campaign.
    pub async fn create_campaign(&self, input: CreateCampaignInput) -> Result<Campaign, AppError> {
        let name = input.name.trim();
        if name.is_empty() || name.chars().count() > 200 {
            return Err(AppError::validation(
                "campaign name must be between 1 and 200 characters",
            ));
        }
        if input.budget_limit.is_some_and(|limit| limit <= 0) {
            return Err(AppError::validation("campaign budget must be positive"));
        }
        validate_campaign_budget(
            input.budget_type,
            input.currency_code.as_ref(),
            input.budget_limit,
        )?;
        validate_window(input.starts_at, input.ends_at, "campaign")?;
        validate_metadata(&input.metadata)?;
        let id = format!("camp_{}", uuid::Uuid::now_v7().simple());
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        sqlx::query(
            "INSERT INTO campaigns
             (id, name, description, budget_type, currency_code, budget_limit, budget_used,
              starts_at, ends_at, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(input.description.as_deref())
        .bind(input.budget_type.as_str())
        .bind(input.currency_code.as_ref().map(ToString::to_string))
        .bind(input.budget_limit)
        .bind(input.starts_at)
        .bind(input.ends_at)
        .bind(input.metadata.to_json_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        self.append_audit(
            &mut tx,
            "campaign.created",
            "campaign",
            &id,
            serde_json::json!({"name": name, "budget_limit": input.budget_limit}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.get_campaign(&id).await
    }

    /// Loads a campaign by identifier.
    pub async fn get_campaign(&self, id: &str) -> Result<Campaign, AppError> {
        let row = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, description, budget_type, currency_code, budget_limit,
                    budget_used, starts_at, ends_at, metadata, created_at, updated_at FROM campaigns WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("campaign", id))?;
        row.into_campaign()
    }

    /// Lists campaigns newest first.
    pub async fn list_campaigns(
        &self,
        pagination: Pagination,
    ) -> Result<PageResult<Campaign>, AppError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM campaigns")
            .fetch_one(self.db.pool())
            .await
            .map_err(database_error)?;
        let rows = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, description, budget_type, currency_code, budget_limit,
                    budget_used, starts_at, ends_at, metadata, created_at, updated_at FROM campaigns
             ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
        )
        .bind(pagination.limit)
        .bind(pagination.offset)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        let data = rows
            .into_iter()
            .map(CampaignRow::into_campaign)
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(PageResult::new(data, count, pagination))
    }

    /// Partially updates a campaign.
    pub async fn update_campaign(
        &self,
        id: &str,
        input: UpdateCampaignInput,
    ) -> Result<Campaign, AppError> {
        let now = now_timestamp();
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let existing_row = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, description, budget_type, currency_code, budget_limit,
                    budget_used, starts_at, ends_at, metadata, created_at, updated_at
             FROM campaigns WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| AppError::not_found("campaign", id))?;
        let existing = existing_row.into_campaign()?;

        let name = input.name.as_deref().map_or_else(
            || Ok(existing.name.clone()),
            |name| {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > 200 {
                    Err(AppError::validation(
                        "campaign name must be between 1 and 200 characters",
                    ))
                } else {
                    Ok(name.to_owned())
                }
            },
        )?;
        let budget_type = input.budget_type.unwrap_or(existing.budget_type);
        if budget_type != existing.budget_type && existing.budget_used > 0 {
            return Err(AppError::conflict(
                "campaign budget type cannot change after usage has been recorded",
            ));
        }
        let currency_code = input
            .currency_code
            .unwrap_or_else(|| existing.currency_code.clone());
        if budget_type == CampaignBudgetType::Spend
            && existing.budget_used > 0
            && currency_code != existing.currency_code
        {
            return Err(AppError::conflict(
                "campaign currency cannot change after spend has been recorded",
            ));
        }
        let budget_limit = input.budget_limit.unwrap_or(existing.budget_limit);
        validate_campaign_budget(budget_type, currency_code.as_ref(), budget_limit)?;
        if budget_limit.is_some_and(|limit| limit < existing.budget_used) {
            return Err(AppError::conflict(
                "campaign budget cannot be below consumed usage",
            ));
        }
        let starts_at = input.starts_at.unwrap_or(existing.starts_at);
        let ends_at = input.ends_at.unwrap_or(existing.ends_at);
        validate_window(starts_at, ends_at, "campaign")?;
        let metadata = input.metadata.unwrap_or_else(|| existing.metadata.clone());
        validate_metadata(&metadata)?;

        let updated = sqlx::query(
            "UPDATE campaigns SET name = ?, description = ?, budget_type = ?, currency_code = ?,
             budget_limit = ?, starts_at = ?, ends_at = ?, metadata = ?, updated_at = ?
             WHERE id = ? AND (? IS NULL OR ? >= budget_used)",
        )
        .bind(&name)
        .bind(
            input
                .description
                .as_deref()
                .or(existing.description.as_deref()),
        )
        .bind(budget_type.as_str())
        .bind(currency_code.as_ref().map(ToString::to_string))
        .bind(budget_limit)
        .bind(starts_at)
        .bind(ends_at)
        .bind(metadata.to_json_string())
        .bind(now)
        .bind(id)
        .bind(budget_limit)
        .bind(budget_limit)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM campaigns WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?;
            if exists == 0 {
                return Err(AppError::not_found("campaign", id));
            }
            return Err(AppError::conflict(
                "campaign budget cannot be below consumed usage",
            ));
        }
        self.append_audit(
            &mut tx,
            "campaign.updated",
            "campaign",
            id,
            serde_json::json!({"name": name, "budget_limit": budget_limit}),
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        self.get_campaign(id).await
    }

    /// Deletes a campaign; promotion links are removed by cascade.
    pub async fn delete_campaign(&self, id: &str) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let result = sqlx::query("DELETE FROM campaigns WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("campaign", id));
        }
        self.append_audit(
            &mut tx,
            "campaign.deleted",
            "campaign",
            id,
            serde_json::Value::Null,
        )
        .await?;
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Links a promotion to a campaign.
    pub async fn link_promotion(
        &self,
        campaign_id: &str,
        promotion_id: &str,
    ) -> Result<(), AppError> {
        self.get_campaign(campaign_id).await?;
        let exists: Option<String> = sqlx::query_scalar("SELECT id FROM promotions WHERE id = ?")
            .bind(promotion_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        if exists.is_none() {
            return Err(AppError::not_found("promotion", promotion_id));
        }
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let linked = sqlx::query(
            "INSERT OR IGNORE INTO campaign_promotions (campaign_id, promotion_id)
             VALUES (?, ?)",
        )
        .bind(campaign_id)
        .bind(promotion_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if linked.rows_affected() > 0 {
            self.append_audit(
                &mut tx,
                "campaign.promotion_linked",
                "campaign",
                campaign_id,
                serde_json::json!({"promotion_id": promotion_id}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Removes a promotion from a campaign.
    pub async fn unlink_promotion(
        &self,
        campaign_id: &str,
        promotion_id: &str,
    ) -> Result<(), AppError> {
        let mut tx = self.db.begin().await.map_err(AppError::from)?;
        let removed = sqlx::query(
            "DELETE FROM campaign_promotions WHERE campaign_id = ? AND promotion_id = ?",
        )
        .bind(campaign_id)
        .bind(promotion_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
        if removed.rows_affected() > 0 {
            self.append_audit(
                &mut tx,
                "campaign.promotion_unlinked",
                "campaign",
                campaign_id,
                serde_json::json!({"promotion_id": promotion_id}),
            )
            .await?;
        }
        tx.commit().await.map_err(database_error)?;
        Ok(())
    }

    /// Lists promotions attached to a campaign.
    pub async fn list_campaign_promotions(
        &self,
        campaign_id: &str,
    ) -> Result<Vec<String>, AppError> {
        self.get_campaign(campaign_id).await?;
        sqlx::query_scalar("SELECT promotion_id FROM campaign_promotions WHERE campaign_id = ?")
            .bind(campaign_id)
            .fetch_all(self.db.pool())
            .await
            .map_err(database_error)
    }

    /// Records one usage and increments promotion, coupon and campaign counters.
    ///
    /// `usage_amount` is the discount granted in minor units and is charged
    /// against monetary campaign budgets. Usage-count campaigns consume one unit
    /// regardless of the amount.
    pub async fn record_usage(
        &self,
        promotion_id: &str,
        coupon_id: Option<&str>,
        customer_id: Option<&str>,
        order_id: Option<&str>,
        usage_amount: i64,
        currency_code: Option<&str>,
    ) -> Result<PromotionUsage, AppError> {
        if usage_amount < 0 {
            return Err(AppError::validation(
                "promotion usage amount must not be negative",
            ));
        }
        let usage_currency = currency_code.map(|code| code.trim().to_ascii_uppercase());
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;

        // The idempotency lookup runs under the same write lock as the insert,
        // so a duplicate order cannot slip in between the two statements.
        if let Some(order_id) = order_id
            && let Some(existing) = sqlx::query_as::<_, PromotionUsageRow>(
                "SELECT id, promotion_id, coupon_id, customer_id, order_id, created_at
                 FROM promotion_usage WHERE promotion_id = ? AND order_id = ?",
            )
            .bind(promotion_id)
            .bind(order_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
        {
            return Ok(existing.into_usage());
        }

        let promotion: Option<(Option<i64>, i64)> =
            sqlx::query_as("SELECT usage_limit, usage_count FROM promotions WHERE id = ?")
                .bind(promotion_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?;
        let Some((usage_limit, usage_count)) = promotion else {
            return Err(AppError::not_found("promotion", promotion_id));
        };
        if usage_limit.is_some_and(|limit| usage_count >= limit) {
            return Err(AppError::conflict("promotion usage limit reached"));
        }

        if let Some(coupon_id) = coupon_id {
            let coupon: Option<(String, Option<i64>, i64)> = sqlx::query_as(
                "SELECT promotion_id, usage_limit, usage_count FROM coupons WHERE id = ?",
            )
            .bind(coupon_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;
            let Some((coupon_promotion_id, coupon_limit, coupon_count)) = coupon else {
                return Err(AppError::not_found("coupon", coupon_id));
            };
            if coupon_promotion_id != promotion_id {
                return Err(AppError::conflict(
                    "coupon does not belong to this promotion",
                ));
            }
            if coupon_limit.is_some_and(|limit| coupon_count >= limit) {
                return Err(AppError::conflict("coupon usage limit reached"));
            }
            let updated = sqlx::query(
                "UPDATE coupons SET usage_count = usage_count + 1, updated_at = ?
                 WHERE id = ? AND (usage_limit IS NULL OR usage_count < usage_limit)",
            )
            .bind(now)
            .bind(coupon_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::conflict("coupon usage limit reached"));
            }
        }

        let id = EventId::new().to_string();
        let inserted = sqlx::query(
            "INSERT INTO promotion_usage
             (id, promotion_id, coupon_id, customer_id, order_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(promotion_id)
        .bind(coupon_id)
        .bind(customer_id)
        .bind(order_id)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = inserted {
            if is_unique_violation(&error)
                && let Some(order_id) = order_id
                && let Some(existing) = sqlx::query_as::<_, PromotionUsageRow>(
                    "SELECT id, promotion_id, coupon_id, customer_id, order_id, created_at
                     FROM promotion_usage WHERE promotion_id = ? AND order_id = ?",
                )
                .bind(promotion_id)
                .bind(order_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
            {
                return Ok(existing.into_usage());
            }
            return Err(database_error(error));
        }
        let updated = sqlx::query(
            "UPDATE promotions SET usage_count = usage_count + 1, updated_at = ?
             WHERE id = ? AND (usage_limit IS NULL OR usage_count < usage_limit)",
        )
        .bind(now)
        .bind(promotion_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() == 0 {
            return Err(AppError::conflict("promotion usage limit reached"));
        }

        let campaigns: Vec<CampaignBudgetRow> = sqlx::query_as(
            "SELECT c.id, c.budget_type, c.currency_code, c.budget_limit, c.budget_used
             FROM campaigns c
             JOIN campaign_promotions cp ON cp.campaign_id = c.id
             WHERE cp.promotion_id = ?",
        )
        .bind(promotion_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        for campaign in campaigns {
            let budget_type: CampaignBudgetType = campaign.budget_type.parse()?;
            let increment = match budget_type {
                CampaignBudgetType::Usage => 1,
                CampaignBudgetType::Spend => {
                    let campaign_currency = campaign.currency_code.ok_or_else(|| {
                        AppError::conflict("spend campaign is missing its currency")
                    })?;
                    let usage_currency = usage_currency.as_deref().ok_or_else(|| {
                        AppError::validation(
                            "a currency is required to consume a spend campaign budget",
                        )
                    })?;
                    if campaign_currency != usage_currency {
                        return Err(AppError::conflict(format!(
                            "campaign currency {campaign_currency} does not match {usage_currency}"
                        )));
                    }
                    usage_amount
                }
            };
            if campaign
                .budget_limit
                .is_some_and(|limit| campaign.budget_used.saturating_add(increment) > limit)
            {
                return Err(AppError::conflict("campaign budget exhausted"));
            }
            let updated = sqlx::query(
                "UPDATE campaigns SET budget_used = budget_used + ?, updated_at = ?
                 WHERE id = ? AND (budget_limit IS NULL OR budget_used + ? <= budget_limit)",
            )
            .bind(increment)
            .bind(now)
            .bind(&campaign.id)
            .bind(increment)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
            if updated.rows_affected() == 0 {
                return Err(AppError::conflict("campaign budget exhausted"));
            }
        }
        transaction.commit().await.map_err(database_error)?;

        Ok(PromotionUsage {
            id,
            promotion_id: promotion_id.to_owned(),
            coupon_id: coupon_id.map(str::to_owned),
            customer_id: customer_id.map(str::to_owned),
            order_id: order_id.map(str::to_owned),
            created_at: now,
        })
    }

    /// Returns the recorded usage count for a promotion.
    pub async fn usage_count(&self, promotion_id: &str) -> Result<i64, AppError> {
        let count: Option<i64> =
            sqlx::query_scalar("SELECT usage_count FROM promotions WHERE id = ?")
                .bind(promotion_id)
                .fetch_optional(self.db.pool())
                .await
                .map_err(database_error)?;
        count.ok_or_else(|| AppError::not_found("promotion", promotion_id))
    }

    /// Returns the recorded usage count for a coupon.
    pub async fn coupon_usage_count(&self, coupon_id: &str) -> Result<i64, AppError> {
        let count: Option<i64> = sqlx::query_scalar("SELECT usage_count FROM coupons WHERE id = ?")
            .bind(coupon_id)
            .fetch_optional(self.db.pool())
            .await
            .map_err(database_error)?;
        count.ok_or_else(|| AppError::not_found("coupon", coupon_id))
    }

    /// Applies eligible active promotions to a cart in creation/id order.
    pub fn calculate(
        &self,
        promotions_with_rules: &[PromotionContext],
        cart: &PromotionCart,
    ) -> Result<Vec<PromotionResult>, AppError> {
        let eligible: Vec<PromotionContext> = promotions_with_rules
            .iter()
            .filter(|context| {
                context.promotion.status == PromotionStatus::Active
                    && window_contains(
                        cart.at,
                        context.promotion.starts_at,
                        context.promotion.ends_at,
                    )
                    && context
                        .promotion
                        .usage_limit
                        .is_none_or(|limit| context.promotion.usage_count < limit)
            })
            .cloned()
            .collect();
        calculate_promotions(&eligible, cart)
    }

    async fn load_promotion_rules(
        &self,
        promotion_id: &str,
    ) -> Result<Vec<PromotionRule>, AppError> {
        let rows = sqlx::query_as::<_, PromotionRuleRow>(&format!(
            "SELECT {PROMOTION_RULE_COLUMNS} FROM promotion_rules
             WHERE promotion_id = ? ORDER BY created_at, id"
        ))
        .bind(promotion_id)
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(PromotionRuleRow::into_rule).collect()
    }

    async fn find_coupon_by_code(&self, code: &str) -> Result<Option<Coupon>, AppError> {
        let row = sqlx::query_as::<_, CouponRow>(&format!(
            "SELECT {COUPON_COLUMNS} FROM coupons WHERE code = ?"
        ))
        .bind(code)
        .fetch_optional(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(row.map(CouponRow::into_coupon))
    }
}

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

async fn ensure_promotion_exists_tx(
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

async fn ensure_coupon_code_available_tx(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    code: &str,
    exclude_id: Option<&str>,
) -> Result<(), AppError> {
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM coupons WHERE code = ?")
        .bind(code)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?;
    if let Some(existing) = existing
        && exclude_id != Some(existing.as_str())
    {
        return Err(AppError::conflict("coupon code is already in use"));
    }
    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => database.is_unique_violation(),
        _ => false,
    }
}

fn unique_or_database(error: sqlx::Error, conflict_message: &str) -> AppError {
    if is_unique_violation(&error) {
        AppError::conflict(conflict_message)
    } else {
        database_error(error)
    }
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

fn push_coupon_filters(builder: &mut QueryBuilder<'_, Sqlite>, filter: &CouponFilter) {
    if let Some(promotion_id) = filter.promotion_id.as_deref() {
        builder
            .push(" AND promotion_id = ")
            .push_bind(promotion_id.to_owned());
    }
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

#[derive(Debug, sqlx::FromRow)]
struct CampaignBudgetRow {
    id: String,
    budget_type: String,
    currency_code: Option<String>,
    budget_limit: Option<i64>,
    budget_used: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct CampaignRow {
    id: String,
    name: String,
    description: Option<String>,
    budget_type: String,
    currency_code: Option<String>,
    budget_limit: Option<i64>,
    budget_used: i64,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl CampaignRow {
    fn into_campaign(self) -> Result<Campaign, AppError> {
        let budget_type = self.budget_type.parse().map_err(|error| {
            AppError::internal(format!("invalid stored campaign budget type: {error}"))
        })?;
        let currency_code = match self.currency_code {
            Some(code) => Some(
                code.parse::<fastcart_kernel::CurrencyCode>()
                    .map_err(|error| {
                        AppError::internal(format!("invalid stored campaign currency: {error}"))
                    })?,
            ),
            None => None,
        };
        Ok(Campaign {
            id: self.id,
            name: self.name,
            description: self.description,
            budget_type,
            currency_code,
            budget_limit: self.budget_limit,
            budget_used: self.budget_used,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
struct PromotionUsageRow {
    id: String,
    promotion_id: String,
    coupon_id: Option<String>,
    customer_id: Option<String>,
    order_id: Option<String>,
    created_at: i64,
}

impl PromotionUsageRow {
    fn into_usage(self) -> PromotionUsage {
        PromotionUsage {
            id: self.id,
            promotion_id: self.promotion_id,
            coupon_id: self.coupon_id,
            customer_id: self.customer_id,
            order_id: self.order_id,
            created_at: self.created_at,
        }
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
            value: self.value,
            status: self.status.parse()?,
            is_automatic: self.is_automatic != 0,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            usage_limit: self.usage_limit,
            usage_count: self.usage_count,
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

#[derive(Debug, sqlx::FromRow)]
struct CouponRow {
    id: String,
    code: String,
    promotion_id: String,
    usage_limit: Option<i64>,
    usage_count: i64,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: String,
    created_at: i64,
    updated_at: i64,
}

impl CouponRow {
    fn into_coupon(self) -> Coupon {
        Coupon {
            id: self.id,
            code: self.code,
            promotion_id: self.promotion_id,
            usage_limit: self.usage_limit,
            usage_count: self.usage_count,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: fastcart_kernel::Metadata::from_json_str(&self.metadata),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
    use fastcart_kernel::{AppError, CurrencyCode, Metadata, Pagination, StoreId};

    use super::PromotionService;
    use crate::domain::{
        CampaignBudgetType, CreateCampaignInput, CreateCouponInput, CreatePromotionInput,
        Promotion, PromotionCart, PromotionContext, PromotionFilter, PromotionLine, PromotionRule,
        PromotionRuleAttribute, PromotionRuleInput, PromotionRuleOperator, PromotionStatus,
        PromotionType, UpdateCampaignInput, UpdatePromotionInput, calculate_promotions,
    };

    async fn service() -> PromotionService {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        PromotionService::new(db)
    }

    fn usd() -> CurrencyCode {
        CurrencyCode::new("USD").unwrap()
    }

    fn eur() -> CurrencyCode {
        CurrencyCode::new("EUR").unwrap()
    }

    fn line(
        line_id: &str,
        product_id: &str,
        variant_id: &str,
        categories: &[&str],
        unit_price: i64,
        quantity: i64,
    ) -> PromotionLine {
        PromotionLine {
            line_id: line_id.to_owned(),
            product_id: product_id.to_owned(),
            variant_id: variant_id.to_owned(),
            category_ids: categories.iter().map(|value| (*value).to_owned()).collect(),
            unit_price,
            quantity,
            is_shipping: false,
        }
    }

    fn cart(subtotal: i64, shipping_total: i64, lines: Vec<PromotionLine>) -> PromotionCart {
        PromotionCart {
            currency: usd(),
            subtotal,
            shipping_total,
            customer_id: Some("cust_1".to_owned()),
            customer_group_ids: Vec::new(),
            lines,
            at: 100,
        }
    }

    fn promotion(
        id: &str,
        promotion_type: PromotionType,
        value: i64,
        created_at: i64,
    ) -> Promotion {
        Promotion {
            id: id.to_owned(),
            code: Some(id.to_ascii_uppercase()),
            title: id.to_owned(),
            promotion_type,
            value,
            status: PromotionStatus::Active,
            is_automatic: false,
            starts_at: None,
            ends_at: None,
            usage_limit: None,
            usage_count: 0,
            metadata: Metadata::empty(),
            rules: Vec::new(),
            created_at,
            updated_at: created_at,
        }
    }

    fn rule(
        attribute: PromotionRuleAttribute,
        operator: PromotionRuleOperator,
        value: &str,
    ) -> PromotionRule {
        PromotionRule {
            id: "rule_1".to_owned(),
            promotion_id: "promo_1".to_owned(),
            attribute,
            operator,
            value: value.to_owned(),
            created_at: 0,
        }
    }

    fn context(promotion: Promotion, rules: Vec<PromotionRule>) -> PromotionContext {
        PromotionContext { promotion, rules }
    }

    #[test]
    fn percentage_rounds_half_away_from_zero() {
        let promotion = promotion("promo_pct", PromotionType::Percentage, 500, 1);
        let cart = cart(
            2020,
            0,
            vec![
                line("item_a", "prod_a", "var_a", &[], 1005, 1),
                line("item_b", "prod_b", "var_b", &[], 1015, 1),
            ],
        );

        let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].total_discount, 101);
        assert_eq!(results[0].applications[0].amount, 50);
        assert_eq!(results[0].applications[1].amount, 51);
        assert_eq!(results[0].shipping_discount, 0);
    }

    #[test]
    fn percentage_targets_a_product() {
        let promotion = promotion("promo_product", PromotionType::Percentage, 1000, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::ProductId,
            PromotionRuleOperator::In,
            "[\"prod_a\"]",
        )];
        let cart = cart(
            3000,
            0,
            vec![
                line("item_a", "prod_a", "var_a", &[], 1000, 1),
                line("item_b", "prod_b", "var_b", &[], 2000, 1),
            ],
        );

        let results = calculate_promotions(&[context(promotion, rules)], &cart).unwrap();
        assert_eq!(results[0].total_discount, 100);
        assert_eq!(results[0].applications.len(), 1);
        assert_eq!(
            results[0].applications[0].line_id.as_deref(),
            Some("item_a")
        );
    }

    #[test]
    fn percentage_targets_a_category() {
        let promotion = promotion("promo_category", PromotionType::FixedAmount, 100, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::CategoryId,
            PromotionRuleOperator::In,
            "[\"cat_sale\"]",
        )];
        let cart = cart(
            1200,
            0,
            vec![
                line("item_a", "prod_a", "var_a", &["cat_sale"], 200, 1),
                line("item_b", "prod_b", "var_b", &["cat_full"], 1000, 1),
            ],
        );

        let results = calculate_promotions(&[context(promotion, rules)], &cart).unwrap();
        assert_eq!(results[0].applications.len(), 1);
        assert_eq!(
            results[0].applications[0].line_id.as_deref(),
            Some("item_a")
        );
        assert_eq!(results[0].applications[0].amount, 100);
    }

    #[test]
    fn fixed_amount_allocates_proportionally_with_last_line_remainder() {
        let promotion = promotion("promo_fixed", PromotionType::FixedAmount, 100, 1);
        let cart = cart(
            300,
            0,
            vec![
                line("item_a", "prod_a", "var_a", &[], 100, 1),
                line("item_b", "prod_b", "var_b", &[], 100, 1),
                line("item_c", "prod_c", "var_c", &[], 100, 1),
            ],
        );

        let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
        let applications = &results[0].applications;
        assert_eq!(applications.len(), 3);
        let amounts: Vec<i64> = applications.iter().map(|item| item.amount).collect();
        assert_eq!(amounts, vec![33, 33, 34]);
        assert_eq!(results[0].total_discount, 100);
    }

    #[test]
    fn fixed_amount_is_capped_at_eligible_subtotal() {
        let promotion = promotion("promo_fixed", PromotionType::FixedAmount, 500, 1);
        let cart = cart(100, 0, vec![line("item_a", "prod_a", "var_a", &[], 100, 1)]);

        let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
        assert_eq!(results[0].total_discount, 100);
        assert_eq!(results[0].applications[0].amount, 100);
    }

    #[test]
    fn free_shipping_discounts_shipping() {
        let promotion = promotion("promo_ship", PromotionType::FreeShipping, 0, 1);
        let cart = cart(
            1000,
            500,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );

        let results = calculate_promotions(&[context(promotion, Vec::new())], &cart).unwrap();
        assert_eq!(results[0].total_discount, 0);
        assert_eq!(results[0].shipping_discount, 500);
        assert_eq!(results[0].applications[0].amount, 500);
        assert!(results[0].applications[0].is_shipping);
        assert!(results[0].applications[0].line_id.is_none());
    }

    #[test]
    fn minimum_subtotal_restricts_application() {
        let promotion = promotion("promo_min", PromotionType::FixedAmount, 100, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::MinimumOrderSubtotal,
            PromotionRuleOperator::Gte,
            "1000",
        )];
        let small = cart(900, 0, vec![line("item_a", "prod_a", "var_a", &[], 900, 1)]);
        let large = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );

        assert!(
            calculate_promotions(&[context(promotion.clone(), rules.clone())], &small)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            calculate_promotions(&[context(promotion, rules)], &large)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn currency_rule_restricts_application() {
        let promotion = promotion("promo_currency", PromotionType::FixedAmount, 100, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::Currency,
            PromotionRuleOperator::Eq,
            "\"EUR\"",
        )];
        let mut usd_cart = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );
        usd_cart.currency = usd();
        let mut eur_cart = usd_cart.clone();
        eur_cart.currency = eur();

        assert!(
            calculate_promotions(&[context(promotion.clone(), rules.clone())], &usd_cart)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            calculate_promotions(&[context(promotion, rules)], &eur_cart)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn customer_group_rule_restricts_application() {
        let promotion = promotion("promo_group", PromotionType::FixedAmount, 100, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::CustomerGroupId,
            PromotionRuleOperator::In,
            "[\"vip\",\"gold\"]",
        )];
        let mut cart = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );
        assert!(
            calculate_promotions(&[context(promotion.clone(), rules.clone())], &cart)
                .unwrap()
                .is_empty()
        );

        cart.customer_group_ids.push("vip".to_owned());
        assert_eq!(
            calculate_promotions(&[context(promotion, rules)], &cart)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn first_order_rule_checks_the_attached_customer() {
        let promotion = promotion("promo_first", PromotionType::FixedAmount, 100, 1);
        let rules = vec![rule(
            PromotionRuleAttribute::FirstOrder,
            PromotionRuleOperator::Eq,
            "true",
        )];
        let mut cart = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );
        assert_eq!(
            calculate_promotions(&[context(promotion.clone(), rules.clone())], &cart)
                .unwrap()
                .len(),
            1
        );

        cart.customer_id = None;
        assert!(
            calculate_promotions(&[context(promotion, rules)], &cart)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn stacking_is_ordered_and_capped_at_remaining_subtotal() {
        let fixed = promotion("promo_a", PromotionType::FixedAmount, 600, 10);
        let percentage = promotion("promo_b", PromotionType::Percentage, 5000, 20);
        let cart = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );

        let results = calculate_promotions(
            &[context(percentage, Vec::new()), context(fixed, Vec::new())],
            &cart,
        )
        .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].promotion_id, "promo_a");
        assert_eq!(results[0].total_discount, 600);
        assert_eq!(results[1].promotion_id, "promo_b");
        assert_eq!(results[1].total_discount, 400);
        assert_eq!(
            results[0].total_discount + results[1].total_discount,
            cart.subtotal
        );
    }

    #[test]
    fn repeated_free_shipping_never_exceeds_shipping_total() {
        let first = promotion("promo_ship_a", PromotionType::FreeShipping, 0, 1);
        let second = promotion("promo_ship_b", PromotionType::FreeShipping, 0, 2);
        let cart = cart(1000, 500, Vec::new());

        let results = calculate_promotions(
            &[context(first, Vec::new()), context(second, Vec::new())],
            &cart,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].shipping_discount, 500);
    }

    #[tokio::test]
    async fn creates_updates_lists_and_deletes_promotions() {
        let service = service().await;
        let input = CreatePromotionInput::new("Ten off", PromotionType::FixedAmount, 1000)
            .active()
            .with_code("tenoff")
            .with_rule(PromotionRuleInput::new(
                PromotionRuleAttribute::MinimumOrderSubtotal,
                PromotionRuleOperator::Gte,
                "5000",
            ));
        let created = service.create_promotion(input).await.unwrap();
        assert_eq!(created.code.as_deref(), Some("TENOFF"));
        assert_eq!(created.rules.len(), 1);
        assert_eq!(created.usage_count, 0);

        let fetched = service.get_promotion(&created.id).await.unwrap();
        assert_eq!(fetched, created);

        service
            .create_promotion(CreatePromotionInput::new(
                "Percentage",
                PromotionType::Percentage,
                1000,
            ))
            .await
            .unwrap();
        let page = service
            .list_promotions(
                PromotionFilter::default(),
                Pagination {
                    limit: 1,
                    offset: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(page.count, 2);
        assert_eq!(page.data.len(), 1);

        let updated = service
            .update_promotion(
                &created.id,
                UpdatePromotionInput {
                    title: Some("Twenty off".to_owned()),
                    value: Some(2000),
                    rules: Some(Vec::new()),
                    ..UpdatePromotionInput::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "Twenty off");
        assert_eq!(updated.value, 2000);
        assert!(updated.rules.is_empty());
        assert_eq!(updated.created_at, created.created_at);

        service.delete_promotion(&created.id).await.unwrap();
        assert!(matches!(
            service.get_promotion(&created.id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn rejects_duplicate_promotion_codes() {
        let service = service().await;
        service
            .create_promotion(
                CreatePromotionInput::new("First", PromotionType::FixedAmount, 100)
                    .with_code("SAVE"),
            )
            .await
            .unwrap();
        let error = service
            .create_promotion(
                CreatePromotionInput::new("Second", PromotionType::FixedAmount, 100)
                    .with_code("save"),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn validates_promotion_configuration() {
        let service = service().await;

        let percentage_error = service
            .create_promotion(CreatePromotionInput::new(
                "Too much",
                PromotionType::Percentage,
                10_001,
            ))
            .await
            .unwrap_err();
        assert!(matches!(percentage_error, AppError::Validation(_)));

        let negative_error = service
            .create_promotion(CreatePromotionInput::new(
                "Negative",
                PromotionType::FixedAmount,
                -1,
            ))
            .await
            .unwrap_err();
        assert!(matches!(negative_error, AppError::Validation(_)));

        let shipping_error = service
            .create_promotion(CreatePromotionInput::new(
                "Shipping value",
                PromotionType::FreeShipping,
                5,
            ))
            .await
            .unwrap_err();
        assert!(matches!(shipping_error, AppError::Validation(_)));

        let window_error = service
            .create_promotion(
                CreatePromotionInput::new("Window", PromotionType::FixedAmount, 100)
                    .with_window(Some(200), Some(100)),
            )
            .await
            .unwrap_err();
        assert!(matches!(window_error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn manages_coupons_with_filters() {
        let service = service().await;
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Coupon promo", PromotionType::FixedAmount, 500).active(),
            )
            .await
            .unwrap();
        let coupon = service
            .create_coupon(CreateCouponInput::new("save10", &promotion.id))
            .await
            .unwrap();
        assert_eq!(coupon.code, "SAVE10");

        let fetched = service.get_coupon(&coupon.id).await.unwrap();
        assert_eq!(fetched, coupon);

        let page = service
            .list_coupons(
                crate::domain::CouponFilter {
                    promotion_id: Some(promotion.id.clone()),
                },
                Pagination::default(),
            )
            .await
            .unwrap();
        assert_eq!(page.count, 1);

        let updated = service
            .update_coupon(
                &coupon.id,
                crate::domain::UpdateCouponInput {
                    usage_limit: Some(Some(2)),
                    ..crate::domain::UpdateCouponInput::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.usage_limit, Some(2));

        service.delete_coupon(&coupon.id).await.unwrap();
        assert!(matches!(
            service.get_coupon(&coupon.id).await.unwrap_err(),
            AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn rejects_duplicate_coupon_codes() {
        let service = service().await;
        let promotion = service
            .create_promotion(CreatePromotionInput::new(
                "Coupon promo",
                PromotionType::FixedAmount,
                500,
            ))
            .await
            .unwrap();
        service
            .create_coupon(CreateCouponInput::new("SAVE", &promotion.id))
            .await
            .unwrap();
        let error = service
            .create_coupon(CreateCouponInput::new("save", &promotion.id))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn validates_coupons_against_windows_limits_and_promotion_state() {
        let service = service().await;
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Coupon promo", PromotionType::FixedAmount, 500).active(),
            )
            .await
            .unwrap();
        let coupon = service
            .create_coupon(
                CreateCouponInput::new("SAVE", &promotion.id).with_window(Some(0), Some(200)),
            )
            .await
            .unwrap();

        assert_eq!(
            service.validate_coupon(&coupon.code, 100).await.unwrap().id,
            coupon.id
        );
        assert!(matches!(
            service
                .validate_coupon(&coupon.code, 200)
                .await
                .unwrap_err(),
            AppError::Conflict(_)
        ));
        assert!(matches!(
            service.validate_coupon("MISSING", 100).await.unwrap_err(),
            AppError::NotFound(_)
        ));

        let draft = service
            .create_promotion(CreatePromotionInput::new(
                "Draft",
                PromotionType::FixedAmount,
                500,
            ))
            .await
            .unwrap();
        service
            .create_coupon(CreateCouponInput::new("DRAFT", &draft.id))
            .await
            .unwrap();
        assert!(matches!(
            service.validate_coupon("DRAFT", 100).await.unwrap_err(),
            AppError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn records_usage_atomically() {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = PromotionService::new(db.clone());
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Usage promo", PromotionType::FixedAmount, 500)
                    .active()
                    .with_usage_limit(5),
            )
            .await
            .unwrap();
        let coupon = service
            .create_coupon(CreateCouponInput::new("USE", &promotion.id))
            .await
            .unwrap();

        let usage = service
            .record_usage(
                &promotion.id,
                Some(&coupon.id),
                Some("cust_1"),
                Some("order_1"),
                0,
                None,
            )
            .await
            .unwrap();
        assert_eq!(usage.promotion_id, promotion.id);
        assert_eq!(usage.coupon_id.as_deref(), Some(coupon.id.as_str()));
        assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
        assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 1);

        let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(recorded, 1);
    }

    #[tokio::test]
    async fn record_usage_is_idempotent_per_order() {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = PromotionService::new(db.clone());
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Idempotent promo", PromotionType::FixedAmount, 500)
                    .active()
                    .with_usage_limit(5),
            )
            .await
            .unwrap();
        let coupon = service
            .create_coupon(CreateCouponInput::new("ONCEPERORDER", &promotion.id))
            .await
            .unwrap();

        let first = service
            .record_usage(
                &promotion.id,
                Some(&coupon.id),
                Some("cust_1"),
                Some("order_1"),
                0,
                None,
            )
            .await
            .unwrap();
        let repeated = service
            .record_usage(
                &promotion.id,
                Some(&coupon.id),
                Some("cust_1"),
                Some("order_1"),
                250,
                None,
            )
            .await
            .unwrap();

        assert_eq!(first.id, repeated.id);
        assert_eq!(service.usage_count(&promotion.id).await.unwrap(), 1);
        assert_eq!(service.coupon_usage_count(&coupon.id).await.unwrap(), 1);
        let recorded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM promotion_usage")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(recorded, 1);
    }

    #[tokio::test]
    async fn duplicate_codes_are_reported_as_conflicts() {
        let service = service().await;
        let first = service
            .create_promotion(
                CreatePromotionInput::new("First", PromotionType::FixedAmount, 100)
                    .with_code("SHARED"),
            )
            .await
            .unwrap();
        let second = service
            .create_promotion(
                CreatePromotionInput::new("Second", PromotionType::FixedAmount, 100)
                    .with_code("OTHER"),
            )
            .await
            .unwrap();
        let error = service
            .update_promotion(
                &second.id,
                UpdatePromotionInput {
                    code: Some(Some("shared".to_owned())),
                    ..UpdatePromotionInput::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));

        let coupon = service
            .create_coupon(CreateCouponInput::new("COUPON-ONE", &first.id))
            .await
            .unwrap();
        service
            .create_coupon(CreateCouponInput::new("COUPON-TWO", &first.id))
            .await
            .unwrap();
        let error = service
            .update_coupon(
                &coupon.id,
                crate::domain::UpdateCouponInput {
                    code: Some("coupon-two".to_owned()),
                    ..crate::domain::UpdateCouponInput::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn usage_limits_are_enforced() {
        let service = service().await;
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Limited", PromotionType::FixedAmount, 500)
                    .active()
                    .with_usage_limit(1),
            )
            .await
            .unwrap();
        service
            .record_usage(&promotion.id, None, None, None, 0, None)
            .await
            .unwrap();
        assert!(matches!(
            service
                .record_usage(&promotion.id, None, None, None, 0, None)
                .await
                .unwrap_err(),
            AppError::Conflict(_)
        ));

        let coupon_promotion = service
            .create_promotion(
                CreatePromotionInput::new("Coupon limited", PromotionType::FixedAmount, 500)
                    .active(),
            )
            .await
            .unwrap();
        let coupon = service
            .create_coupon(CreateCouponInput::new("ONCE", &coupon_promotion.id).with_usage_limit(1))
            .await
            .unwrap();
        service
            .record_usage(&coupon_promotion.id, Some(&coupon.id), None, None, 0, None)
            .await
            .unwrap();
        assert!(matches!(
            service
                .record_usage(&coupon_promotion.id, Some(&coupon.id), None, None, 0, None)
                .await
                .unwrap_err(),
            AppError::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn lists_eligible_automatic_promotions() {
        let service = service().await;
        let eligible = service
            .create_promotion(
                CreatePromotionInput::new("Automatic", PromotionType::FixedAmount, 100)
                    .active()
                    .automatic()
                    .with_window(Some(0), Some(1000)),
            )
            .await
            .unwrap();
        service
            .create_promotion(
                CreatePromotionInput::new("Draft automatic", PromotionType::FixedAmount, 100)
                    .automatic(),
            )
            .await
            .unwrap();
        service
            .create_promotion(
                CreatePromotionInput::new("Manual", PromotionType::FixedAmount, 100).active(),
            )
            .await
            .unwrap();
        service
            .create_promotion(
                CreatePromotionInput::new("Expired", PromotionType::FixedAmount, 100)
                    .active()
                    .automatic()
                    .with_window(Some(0), Some(50)),
            )
            .await
            .unwrap();

        let automatic = service.list_automatic_promotions(100).await.unwrap();
        assert_eq!(automatic.len(), 1);
        assert_eq!(automatic[0].id, eligible.id);
        assert_eq!(automatic[0].rules.len(), 0);
    }

    #[tokio::test]
    async fn service_calculation_filters_inactive_promotions() {
        let service = service().await;
        let active = promotion("promo_active", PromotionType::FixedAmount, 100, 1);
        let mut draft = promotion("promo_draft", PromotionType::FixedAmount, 200, 2);
        draft.status = PromotionStatus::Draft;
        let mut expired = promotion("promo_expired", PromotionType::FixedAmount, 300, 3);
        expired.ends_at = Some(50);
        let cart = cart(
            1000,
            0,
            vec![line("item_a", "prod_a", "var_a", &[], 1000, 1)],
        );

        let results = service
            .calculate(
                &[
                    context(active, Vec::new()),
                    context(draft, Vec::new()),
                    context(expired, Vec::new()),
                ],
                &cart,
            )
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].promotion_id, "promo_active");
    }

    #[tokio::test]
    async fn campaign_mutations_append_audit_entries_atomically() {
        let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
        let service = PromotionService::new(db.clone()).with_audit_actor(Some("user_1".to_owned()));

        let campaign = service
            .create_campaign(CreateCampaignInput {
                name: "Launch".to_owned(),
                ..CreateCampaignInput::default()
            })
            .await
            .unwrap();
        service
            .update_campaign(
                &campaign.id,
                UpdateCampaignInput {
                    name: Some("Launch 2".to_owned()),
                    ..UpdateCampaignInput::default()
                },
            )
            .await
            .unwrap();
        service.delete_campaign(&campaign.id).await.unwrap();

        let audits = fetch_pending_audits(db.pool(), 10).await.unwrap();
        assert_eq!(
            audits
                .iter()
                .map(|audit| audit.action.as_str())
                .collect::<Vec<_>>(),
            vec!["campaign.created", "campaign.updated", "campaign.deleted"]
        );
        assert!(audits.iter().all(|audit| audit.processed_at.is_none()));
        assert!(
            audits
                .iter()
                .all(|audit| audit.actor_id.as_deref() == Some("user_1"))
        );

        // A failed deletion is a no-op and must not append an audit row.
        assert!(service.delete_campaign(&campaign.id).await.is_err());
        assert_eq!(fetch_pending_audits(db.pool(), 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn campaign_budgets_are_consumed_atomically() {
        let service = service().await;
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Budgeted", PromotionType::FixedAmount, 100).active(),
            )
            .await
            .unwrap();
        let campaign = service
            .create_campaign(CreateCampaignInput {
                name: "Launch".to_owned(),
                budget_limit: Some(1),
                ..CreateCampaignInput::default()
            })
            .await
            .unwrap();
        service
            .link_promotion(&campaign.id, &promotion.id)
            .await
            .unwrap();
        assert_eq!(
            service
                .list_campaign_promotions(&campaign.id)
                .await
                .unwrap(),
            vec![promotion.id.clone()]
        );

        service
            .record_usage(&promotion.id, None, None, Some("order_1"), 0, None)
            .await
            .unwrap();
        let exhausted = service
            .record_usage(&promotion.id, None, None, Some("order_2"), 0, None)
            .await
            .unwrap_err();
        assert!(matches!(exhausted, AppError::Conflict(_)));
        let stored = service.get_campaign(&campaign.id).await.unwrap();
        assert_eq!(stored.budget_used, 1);
    }

    #[tokio::test]
    async fn spend_campaign_budgets_track_monetary_amounts() {
        let service = service().await;
        let promotion = service
            .create_promotion(
                CreatePromotionInput::new("Spend budget", PromotionType::Percentage, 1000).active(),
            )
            .await
            .unwrap();
        let campaign = service
            .create_campaign(CreateCampaignInput {
                name: "Spend".to_owned(),
                budget_type: CampaignBudgetType::Spend,
                currency_code: Some(usd()),
                budget_limit: Some(1_000),
                ..CreateCampaignInput::default()
            })
            .await
            .unwrap();
        service
            .link_promotion(&campaign.id, &promotion.id)
            .await
            .unwrap();

        service
            .record_usage(&promotion.id, None, None, Some("order_1"), 600, Some("USD"))
            .await
            .unwrap();
        let stored = service.get_campaign(&campaign.id).await.unwrap();
        assert_eq!(stored.budget_used, 600);
        assert_eq!(stored.budget_spent().unwrap().amount_minor(), 600);

        let wrong_currency = service
            .record_usage(&promotion.id, None, None, Some("order_2"), 100, Some("EUR"))
            .await
            .unwrap_err();
        assert!(matches!(wrong_currency, AppError::Conflict(_)));

        let exhausted = service
            .record_usage(&promotion.id, None, None, Some("order_3"), 500, Some("USD"))
            .await
            .unwrap_err();
        assert!(matches!(exhausted, AppError::Conflict(_)));
        assert_eq!(
            service
                .get_campaign(&campaign.id)
                .await
                .unwrap()
                .budget_used,
            600
        );

        service
            .record_usage(&promotion.id, None, None, Some("order_4"), 400, Some("USD"))
            .await
            .unwrap();
        assert_eq!(
            service
                .get_campaign(&campaign.id)
                .await
                .unwrap()
                .budget_used,
            1_000
        );
    }
}
