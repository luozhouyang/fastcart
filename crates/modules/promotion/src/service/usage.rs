//! Promotion usage, reservation and release.

//! SQLite-backed promotion service.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{AppError, EventId, now_timestamp};
use sqlx::Sqlite;

use crate::domain::{CampaignBudgetType, PromotionUsage};

use super::campaigns::CampaignBudgetRow;
/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
use super::*;

/// Loads the campaigns a promotion belongs to, with their budget state.
async fn load_promotion_campaigns(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    promotion_id: &str,
) -> Result<Vec<CampaignBudgetRow>, AppError> {
    sqlx::query_as(
        "SELECT c.id, c.budget_type, c.currency_code, c.budget_limit, c.budget_used
         FROM campaigns c
         JOIN campaign_promotions cp ON cp.campaign_id = c.id
         WHERE cp.promotion_id = ?",
    )
    .bind(promotion_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)
}

/// Moves the recorded charge of a claim onto the order that owns it now.
///
/// A charge already recorded for the order wins; the checkout copy is dropped so
/// a release can never reverse the same budget twice.
async fn move_usage_application(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    promotion_id: &str,
    from_key: &str,
    to_order_id: &str,
) -> Result<(), AppError> {
    let already_recorded: Option<String> = sqlx::query_scalar(
        "SELECT id FROM promotion_applications
         WHERE promotion_id = ? AND order_id = ? LIMIT 1",
    )
    .bind(promotion_id)
    .bind(to_order_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    if already_recorded.is_some() {
        return delete_usage_application(transaction, promotion_id, from_key).await;
    }
    sqlx::query(
        "UPDATE promotion_applications SET order_id = ? WHERE promotion_id = ? AND order_id = ?",
    )
    .bind(to_order_id)
    .bind(promotion_id)
    .bind(from_key)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

/// Forgets the recorded charge of a claim that no longer exists.
async fn delete_usage_application(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    promotion_id: &str,
    key: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM promotion_applications WHERE promotion_id = ? AND order_id = ?")
        .bind(promotion_id)
        .bind(key)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(())
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

impl PromotionService {
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
        self.record_usage_keyed(
            promotion_id,
            coupon_id,
            customer_id,
            order_id,
            usage_amount,
            currency_code,
        )
        .await
    }

    /// Atomically claims one usage for a checkout before the money moves.
    ///
    /// The claim is recorded under the checkout-scoped key
    /// (`checkout:<checkout_id>`) so it can be moved onto the order once it
    /// exists ([`Self::reassign_usage`]) or given back if it never does
    /// ([`Self::release_usage`]). Claiming reuses the recording path, so it
    /// validates and charges the promotion, coupon, per-customer and campaign
    /// counters in one `BEGIN IMMEDIATE` transaction; a caller that loses the
    /// race gets a conflict instead of an over-issued discount. Repeating the
    /// call for the same checkout returns the existing claim and counts once.
    pub async fn reserve_usage(
        &self,
        promotion_id: &str,
        coupon_id: Option<&str>,
        customer_id: Option<&str>,
        checkout_id: &str,
        usage_amount: i64,
        currency_code: Option<&str>,
    ) -> Result<PromotionUsage, AppError> {
        let checkout_id = checkout_id.trim();
        if checkout_id.is_empty() {
            return Err(AppError::validation(
                "a checkout id is required to reserve a promotion usage",
            ));
        }
        let key = checkout_usage_key(checkout_id);
        self.record_usage_keyed(
            promotion_id,
            coupon_id,
            customer_id,
            Some(&key),
            usage_amount,
            currency_code,
        )
        .await
    }

    /// Moves the usage a checkout claimed onto the order that was placed.
    ///
    /// The counters are left alone: the usage was charged when it was claimed,
    /// and moving the key must not charge — or refund — a second time. A claim
    /// whose promotion already records the order is dropped instead of
    /// duplicated, so a takeover or a replay converges on one row. Returns the
    /// number of claims the key held.
    pub async fn reassign_usage(&self, from_key: &str, to_order_id: &str) -> Result<u64, AppError> {
        let from_key = from_key.trim();
        if from_key.is_empty() {
            return Err(AppError::validation(
                "a checkout usage key is required to reassign a promotion usage",
            ));
        }
        let to_order_id = to_order_id.trim();
        if to_order_id.is_empty() {
            return Err(AppError::validation(
                "an order id is required to reassign a promotion usage",
            ));
        }
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let claims: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, promotion_id FROM promotion_usage WHERE order_id = ? ORDER BY id",
        )
        .bind(from_key)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;

        let mut moved = 0_u64;
        for (id, promotion_id) in claims {
            let already_recorded: Option<String> = sqlx::query_scalar(
                "SELECT id FROM promotion_usage WHERE promotion_id = ? AND order_id = ? LIMIT 1",
            )
            .bind(&promotion_id)
            .bind(to_order_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;
            if already_recorded.is_some() {
                sqlx::query("DELETE FROM promotion_usage WHERE id = ?")
                    .bind(&id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(database_error)?;
                delete_usage_application(&mut transaction, &promotion_id, from_key).await?;
            } else {
                sqlx::query("UPDATE promotion_usage SET order_id = ? WHERE id = ?")
                    .bind(to_order_id)
                    .bind(&id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(database_error)?;
                move_usage_application(&mut transaction, &promotion_id, from_key, to_order_id)
                    .await?;
            }
            moved += 1;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(moved)
    }

    /// Checkouts that still hold a usage claim, without the `checkout:` prefix.
    ///
    /// Reads only this module's own table, so the caller — the maintenance
    /// workflow, which is allowed to see several modules at once — can decide
    /// which of those checkouts never became an order.
    pub async fn claimed_checkout_ids(&self) -> Result<Vec<String>, AppError> {
        let keys: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT order_id FROM promotion_usage WHERE order_id LIKE 'checkout:%'",
        )
        .fetch_all(self.db.pool())
        .await
        .map_err(database_error)?;
        Ok(keys
            .into_iter()
            .map(|key| checkout_usage_key_strip(&key).to_owned())
            .collect())
    }

    /// Releases the claims of checkouts the caller has established are dead.
    ///
    /// A claim is taken before the capture and moved onto the order once it is
    /// placed. A checkout that never gets that far keeps the redemption for
    /// good, so the promotion stays consumed for a discount nobody received.
    ///
    /// Which checkouts are dead is the caller's judgement, not this module's:
    /// that knowledge lives in the checkout table, which this module must not
    /// read.
    pub async fn release_abandoned_usage(
        &self,
        abandoned_checkout_ids: &[String],
    ) -> Result<u64, AppError> {
        let mut released = 0;
        for checkout_id in abandoned_checkout_ids {
            released += self.release_usage(&checkout_usage_key(checkout_id)).await?;
        }
        Ok(released)
    }

    /// Gives every usage a checkout claimed back, counters included.
    ///
    /// Deletes the `checkout:<checkout_id>` rows and reverses what the claim
    /// charged in the same transaction: `promotions.usage_count`,
    /// `coupons.usage_count` and the campaign budget the claim consumed. Every
    /// counter is floored at zero, so releasing twice cannot drive one
    /// negative. Returns the number of released claims.
    pub async fn release_usage(&self, checkout_key: &str) -> Result<u64, AppError> {
        let checkout_key = checkout_key.trim();
        if checkout_key.is_empty() {
            return Err(AppError::validation(
                "a checkout usage key is required to release a promotion usage",
            ));
        }
        let now = now_timestamp();
        let mut transaction = self.db.begin().await.map_err(AppError::from)?;
        let claims: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT id, promotion_id, coupon_id FROM promotion_usage WHERE order_id = ? ORDER BY id",
        )
        .bind(checkout_key)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;

        let mut released = 0_u64;
        for (id, promotion_id, coupon_id) in claims {
            // The amount and currency the claim charged are recorded next to the
            // usage row; `promotion_usage` itself has no amount column.
            let charge: Option<(i64, Option<String>)> = sqlx::query_as(
                "SELECT amount, currency_code FROM promotion_applications
                 WHERE promotion_id = ? AND order_id = ? ORDER BY id LIMIT 1",
            )
            .bind(&promotion_id)
            .bind(checkout_key)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;

            for campaign in load_promotion_campaigns(&mut transaction, &promotion_id).await? {
                let budget_type: CampaignBudgetType = campaign.budget_type.parse()?;
                let decrement = match budget_type {
                    CampaignBudgetType::Usage => 1,
                    CampaignBudgetType::Spend => {
                        let Some((amount, currency)) = charge.as_ref() else {
                            tracing::warn!(
                                promotion_id = %promotion_id,
                                campaign_id = %campaign.id,
                                "no recorded amount for a claimed spend campaign budget; leaving it consumed"
                            );
                            continue;
                        };
                        if currency.as_deref() != campaign.currency_code.as_deref() {
                            tracing::warn!(
                                promotion_id = %promotion_id,
                                campaign_id = %campaign.id,
                                "recorded currency does not match the campaign; leaving its budget consumed"
                            );
                            continue;
                        }
                        *amount
                    }
                };
                sqlx::query(
                    "UPDATE campaigns SET budget_used = MAX(0, budget_used - ?), updated_at = ?
                     WHERE id = ?",
                )
                .bind(decrement)
                .bind(now)
                .bind(&campaign.id)
                .execute(&mut *transaction)
                .await
                .map_err(database_error)?;
            }
            if let Some(coupon_id) = coupon_id.as_deref() {
                sqlx::query(
                    "UPDATE coupons SET usage_count = MAX(0, usage_count - 1), updated_at = ?
                     WHERE id = ?",
                )
                .bind(now)
                .bind(coupon_id)
                .execute(&mut *transaction)
                .await
                .map_err(database_error)?;
            }
            sqlx::query(
                "UPDATE promotions SET usage_count = MAX(0, usage_count - 1), updated_at = ?
                 WHERE id = ?",
            )
            .bind(now)
            .bind(&promotion_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
            sqlx::query("DELETE FROM promotion_usage WHERE id = ?")
                .bind(&id)
                .execute(&mut *transaction)
                .await
                .map_err(database_error)?;
            sqlx::query(
                "DELETE FROM promotion_applications WHERE promotion_id = ? AND order_id = ?",
            )
            .bind(&promotion_id)
            .bind(checkout_key)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
            released += 1;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(released)
    }

    /// Shared insert path of [`Self::record_usage`] and [`Self::reserve_usage`].
    ///
    /// `order_key` is the order id for a recorded usage and the checkout key for
    /// a claimed one. The caller owns the transaction, so the limit checks, the
    /// counter updates and the insert all run under one write lock.
    async fn record_usage_keyed(
        &self,
        promotion_id: &str,
        coupon_id: Option<&str>,
        customer_id: Option<&str>,
        order_key: Option<&str>,
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
        if let Some(order_key) = order_key
            && let Some(existing) = sqlx::query_as::<_, PromotionUsageRow>(
                "SELECT id, promotion_id, coupon_id, customer_id, order_id, created_at
                 FROM promotion_usage WHERE promotion_id = ? AND order_id = ?",
            )
            .bind(promotion_id)
            .bind(order_key)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
        {
            return Ok(existing.into_usage());
        }

        let promotion: Option<(Option<i64>, i64, Option<i64>)> = sqlx::query_as(
            "SELECT usage_limit, usage_count, per_customer_limit FROM promotions WHERE id = ?",
        )
        .bind(promotion_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let Some((usage_limit, usage_count, per_customer_limit)) = promotion else {
            return Err(AppError::not_found("promotion", promotion_id));
        };
        if usage_limit.is_some_and(|limit| usage_count >= limit) {
            return Err(AppError::conflict("promotion usage limit reached"));
        }
        if let (Some(limit), Some(customer_id)) = (per_customer_limit, customer_id) {
            // Counted under the same write lock as the insert, so two checkouts
            // of one customer cannot both pass the check.
            let recorded: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM promotion_usage WHERE promotion_id = ? AND customer_id = ?",
            )
            .bind(promotion_id)
            .bind(customer_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?;
            if recorded >= limit {
                return Err(AppError::conflict(
                    "promotion per-customer usage limit reached",
                ));
            }
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
        .bind(order_key)
        .bind(now)
        .execute(&mut *transaction)
        .await;
        if let Err(error) = inserted {
            if is_unique_violation(&error)
                && let Some(order_key) = order_key
                && let Some(existing) = sqlx::query_as::<_, PromotionUsageRow>(
                    "SELECT id, promotion_id, coupon_id, customer_id, order_id, created_at
                     FROM promotion_usage WHERE promotion_id = ? AND order_id = ?",
                )
                .bind(promotion_id)
                .bind(order_key)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
            {
                return Ok(existing.into_usage());
            }
            return Err(database_error(error));
        }
        // The usage row carries no amount, so the charge is recorded here:
        // `release_usage` reverses a claimed campaign budget from this row. The
        // column is `NOT NULL`, and a spend campaign always requires a currency,
        // so an absent one is stored empty.
        sqlx::query(
            "INSERT INTO promotion_applications
             (id, promotion_id, cart_id, order_id, code, amount, currency_code, created_at)
             VALUES (?, ?, NULL, ?, NULL, ?, ?, ?)",
        )
        .bind(EventId::new().to_string())
        .bind(promotion_id)
        .bind(order_key)
        .bind(usage_amount)
        .bind(usage_currency.as_deref().unwrap_or_default())
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
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

        for campaign in load_promotion_campaigns(&mut transaction, promotion_id).await? {
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
            order_id: order_key.map(str::to_owned),
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
}
