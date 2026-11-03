//! Admin promotion routes: campaigns, coupons and usage accounting.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiQuery, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, Metadata, now_timestamp};
use fastcart_module_promotion::{
    Coupon, CouponFilter, CreateCouponInput, CreatePromotionInput, Promotion, PromotionFilter,
    PromotionRuleAttribute, PromotionRuleInput, PromotionRuleOperator, PromotionService,
    PromotionStatus, PromotionType, UpdateCouponInput, UpdatePromotionInput,
};
use serde::{Deserialize, Serialize};

use crate::dto::PageResponse;
use crate::routes::StoreDb;
use crate::state::AppContext;

const READ_PERMISSION: &str = "promotions.read";
const WRITE_PERMISSION: &str = "promotions.write";

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/promotions", get(list_promotions).post(create_promotion))
        .route(
            "/promotions/{id}",
            get(get_promotion)
                .patch(update_promotion)
                .delete(delete_promotion),
        )
        .route("/promotions/{id}/usage", get(promotion_usage))
        .route("/coupons", get(list_coupons).post(create_coupon))
        .route("/coupons/validate", post(validate_coupon))
        .route(
            "/coupons/{id}",
            get(get_coupon).patch(update_coupon).delete(delete_coupon),
        )
}

// Promotions

async fn list_promotions(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<PromotionListQuery>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Promotion>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = PromotionService::new(db)
        .list_promotions(query.into_filter(), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_promotion(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreatePromotionRequest>,
) -> ApiResult<(StatusCode, Json<Promotion>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let promotion = PromotionService::new(db)
        .create_promotion(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(promotion)))
}

async fn get_promotion(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Promotion>> {
    require_permission(&actor, READ_PERMISSION)?;
    let promotion = PromotionService::new(db).get_promotion(&id).await?;
    Ok(Json(promotion))
}

async fn update_promotion(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdatePromotionRequest>,
) -> ApiResult<Json<Promotion>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let promotion = PromotionService::new(db)
        .update_promotion(&id, request.into_input())
        .await?;
    Ok(Json(promotion))
}

async fn delete_promotion(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    PromotionService::new(db).delete_promotion(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn promotion_usage(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PromotionUsageResponse>> {
    require_permission(&actor, READ_PERMISSION)?;
    let usage_count = PromotionService::new(db).usage_count(&id).await?;
    Ok(Json(PromotionUsageResponse { usage_count }))
}

// Coupons

async fn list_coupons(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<CouponListQuery>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Coupon>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = PromotionService::new(db)
        .list_coupons(query.into_filter(), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_coupon(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCouponRequest>,
) -> ApiResult<(StatusCode, Json<Coupon>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let coupon = PromotionService::new(db)
        .create_coupon(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(coupon)))
}

async fn get_coupon(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Coupon>> {
    require_permission(&actor, READ_PERMISSION)?;
    let coupon = PromotionService::new(db).get_coupon(&id).await?;
    Ok(Json(coupon))
}

async fn update_coupon(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCouponRequest>,
) -> ApiResult<Json<Coupon>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let coupon = PromotionService::new(db)
        .update_coupon(&id, request.into_input())
        .await?;
    Ok(Json(coupon))
}

async fn delete_coupon(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    PromotionService::new(db).delete_coupon(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_coupon(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<ValidateCouponRequest>,
) -> ApiResult<Json<CouponValidationResponse>> {
    require_permission(&actor, READ_PERMISSION)?;
    // Invalid, expired or exhausted coupons surface as the service's
    // `NotFound`/`Conflict` errors (rendered as 404/409), never as `valid: false`.
    let coupon = PromotionService::new(db)
        .validate_coupon(&request.code, now_timestamp())
        .await?;
    Ok(Json(CouponValidationResponse {
        valid: true,
        coupon: Some(coupon),
    }))
}

// Helpers

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

/// Distinguishes an absent field from an explicit JSON `null`.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct PromotionListQuery {
    status: Option<PromotionStatus>,
    is_automatic: Option<bool>,
    promotion_type: Option<PromotionType>,
}

impl PromotionListQuery {
    fn into_filter(self) -> PromotionFilter {
        PromotionFilter {
            status: self.status,
            is_automatic: self.is_automatic,
            promotion_type: self.promotion_type,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PromotionRuleRequest {
    attribute: PromotionRuleAttribute,
    operator: Option<PromotionRuleOperator>,
    value: String,
}

impl From<PromotionRuleRequest> for PromotionRuleInput {
    fn from(request: PromotionRuleRequest) -> Self {
        Self {
            attribute: request.attribute,
            operator: request.operator.unwrap_or(PromotionRuleOperator::Eq),
            value: request.value,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePromotionRequest {
    code: Option<String>,
    title: String,
    promotion_type: PromotionType,
    value: i64,
    status: Option<PromotionStatus>,
    is_automatic: Option<bool>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    usage_limit: Option<i64>,
    metadata: Option<Metadata>,
    #[serde(default)]
    rules: Vec<PromotionRuleRequest>,
}

impl CreatePromotionRequest {
    fn into_input(self) -> CreatePromotionInput {
        let mut input = CreatePromotionInput::new(self.title, self.promotion_type, self.value);
        input.code = self.code;
        input.status = self.status.unwrap_or(PromotionStatus::Draft);
        input.is_automatic = self.is_automatic.unwrap_or(false);
        input.starts_at = self.starts_at;
        input.ends_at = self.ends_at;
        input.usage_limit = self.usage_limit;
        input.metadata = self.metadata.unwrap_or_default();
        input.rules = self.rules.into_iter().map(Into::into).collect();
        input
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePromotionRequest {
    #[serde(default, deserialize_with = "nullable")]
    code: Option<Option<String>>,
    title: Option<String>,
    promotion_type: Option<PromotionType>,
    value: Option<i64>,
    status: Option<PromotionStatus>,
    is_automatic: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    ends_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    usage_limit: Option<Option<i64>>,
    metadata: Option<Metadata>,
    rules: Option<Vec<PromotionRuleRequest>>,
}

impl UpdatePromotionRequest {
    fn into_input(self) -> UpdatePromotionInput {
        UpdatePromotionInput {
            code: self.code,
            title: self.title,
            promotion_type: self.promotion_type,
            value: self.value,
            status: self.status,
            is_automatic: self.is_automatic,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            usage_limit: self.usage_limit,
            metadata: self.metadata,
            rules: self
                .rules
                .map(|rules| rules.into_iter().map(Into::into).collect()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct CouponListQuery {
    promotion_id: Option<String>,
}

impl CouponListQuery {
    fn into_filter(self) -> CouponFilter {
        CouponFilter {
            promotion_id: self.promotion_id,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCouponRequest {
    code: String,
    promotion_id: String,
    usage_limit: Option<i64>,
    starts_at: Option<i64>,
    ends_at: Option<i64>,
    metadata: Option<Metadata>,
}

impl CreateCouponRequest {
    fn into_input(self) -> CreateCouponInput {
        CreateCouponInput {
            code: self.code,
            promotion_id: self.promotion_id,
            usage_limit: self.usage_limit,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: self.metadata.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateCouponRequest {
    code: Option<String>,
    promotion_id: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    usage_limit: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "nullable")]
    ends_at: Option<Option<i64>>,
    metadata: Option<Metadata>,
}

impl UpdateCouponRequest {
    fn into_input(self) -> UpdateCouponInput {
        UpdateCouponInput {
            code: self.code,
            promotion_id: self.promotion_id,
            usage_limit: self.usage_limit,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            metadata: self.metadata,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ValidateCouponRequest {
    code: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[schema(as = CouponValidation)]
pub struct CouponValidationResponse {
    valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    coupon: Option<Coupon>,
}

#[derive(Debug, Serialize)]
struct PromotionUsageResponse {
    usage_count: i64,
}

#[cfg(test)]
mod tests {
    use super::UpdatePromotionRequest;

    #[test]
    fn nullable_code_distinguishes_null_from_absence() {
        let absent: UpdatePromotionRequest = serde_json::from_str("{}").expect("request");
        assert_eq!(absent.code, None);

        let cleared: UpdatePromotionRequest =
            serde_json::from_str(r#"{"code":null}"#).expect("request");
        assert_eq!(cleared.code, Some(None));

        let replaced: UpdatePromotionRequest =
            serde_json::from_str(r#"{"code":"SAVE"}"#).expect("request");
        assert_eq!(replaced.code, Some(Some("SAVE".to_owned())));
    }
}
