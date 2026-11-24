//! Coupon authoring and validation.

//! SQLite-backed promotion service.

use fastcart_infrastructure::database_error;
use fastcart_kernel::{
    AppError, CouponId, PageResult, Pagination, now_timestamp, validate_metadata,
};
use sqlx::{QueryBuilder, Sqlite};

use crate::domain::{
    Coupon, CouponFilter, CreateCouponInput, PromotionStatus, UpdateCouponInput, normalize_code,
    validate_usage_limit, validate_window, window_contains,
};

use super::promotions::ensure_promotion_exists_tx;
/// Application service for promotions, coupons, usage accounting and pricing
/// calculations.
use super::*;
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

fn push_coupon_filters(builder: &mut QueryBuilder<'_, Sqlite>, filter: &CouponFilter) {
    if let Some(promotion_id) = filter.promotion_id.as_deref() {
        builder
            .push(" AND promotion_id = ")
            .push_bind(promotion_id.to_owned());
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

impl PromotionService {
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
