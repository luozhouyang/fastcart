//! Campaigns and promotion links.

//! SQLite-backed promotion service.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, PageResult, Pagination, now_timestamp, validate_metadata};

use crate::domain::{
    Campaign, CampaignBudgetType, CreateCampaignInput, UpdateCampaignInput,
    validate_campaign_budget, validate_window,
};

/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
use super::*;

#[derive(Debug, sqlx::FromRow)]
pub(super) struct CampaignBudgetRow {
    pub(super) id: String,
    pub(super) budget_type: String,
    pub(super) currency_code: Option<String>,
    pub(super) budget_limit: Option<i64>,
    pub(super) budget_used: i64,
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

impl PromotionService {
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
}
