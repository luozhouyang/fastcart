//! Admin campaign and budget routes.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, CurrencyCode, Metadata};
use fastcart_module_promotion::{
    Campaign, CampaignBudgetType, CreateCampaignInput, PromotionService, UpdateCampaignInput,
};
use serde::{Deserialize, Serialize};

use super::{StoreDb, record_store_audit};
use crate::dto::{PageInfo, PageResponse};
use crate::state::AppContext;

/// Builds campaign routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/campaigns", get(list_campaigns).post(create_campaign))
        .route(
            "/campaigns/{id}",
            get(get_campaign)
                .patch(update_campaign)
                .delete(delete_campaign),
        )
        .route(
            "/campaigns/{id}/promotions",
            get(list_promotions).post(link_promotion),
        )
        .route(
            "/campaigns/{id}/promotions/{promotion_id}",
            delete(unlink_promotion),
        )
}

/// Campaign response body.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CampaignDto {
    /// Campaign identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Budget accounting mode.
    pub budget_type: CampaignBudgetType,
    /// Currency of a monetary budget.
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units.
    pub budget_limit: Option<i64>,
    /// Usages or spend already consumed.
    pub budget_used: i64,
    /// Start timestamp.
    pub starts_at: Option<i64>,
    /// End timestamp.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<Campaign> for CampaignDto {
    fn from(campaign: Campaign) -> Self {
        Self {
            id: campaign.id,
            name: campaign.name,
            description: campaign.description,
            budget_type: campaign.budget_type,
            currency_code: campaign.currency_code,
            budget_limit: campaign.budget_limit,
            budget_used: campaign.budget_used,
            starts_at: campaign.starts_at,
            ends_at: campaign.ends_at,
            metadata: campaign.metadata,
            created_at: campaign.created_at,
            updated_at: campaign.updated_at,
        }
    }
}

/// Campaign creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCampaignRequest {
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Budget accounting mode; defaults to usage.
    #[serde(default)]
    pub budget_type: CampaignBudgetType,
    /// Currency required by spend budgets.
    #[serde(default)]
    pub currency_code: Option<CurrencyCode>,
    /// Maximum usages or spend in minor units.
    pub budget_limit: Option<i64>,
    /// Start timestamp.
    pub starts_at: Option<i64>,
    /// End timestamp.
    pub ends_at: Option<i64>,
    /// Extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

/// Campaign update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateCampaignRequest {
    /// New name.
    pub name: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New budget accounting mode.
    pub budget_type: Option<CampaignBudgetType>,
    /// New budget currency; `null` clears it.
    pub currency_code: Option<Option<CurrencyCode>>,
    /// New budget; `null` makes it unlimited.
    pub budget_limit: Option<Option<i64>>,
    /// New start; `null` clears it.
    pub starts_at: Option<Option<i64>>,
    /// New end; `null` clears it.
    pub ends_at: Option<Option<i64>>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Promotion link body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct LinkPromotionRequest {
    /// Promotion identifier.
    pub promotion_id: String,
}

fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

async fn list_campaigns(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<CampaignDto>>> {
    authorize(&actor, "promotions.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = PromotionService::new(db).list_campaigns(pagination).await?;
    Ok(Json(PageResponse {
        data: page.data.into_iter().map(CampaignDto::from).collect(),
        pagination: PageInfo {
            limit: page.pagination.limit,
            offset: page.pagination.offset,
            total: page.count,
        },
    }))
}

async fn get_campaign(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<CampaignDto>> {
    authorize(&actor, "promotions.read")?;
    Ok(Json(CampaignDto::from(
        PromotionService::new(db).get_campaign(&id).await?,
    )))
}

async fn create_campaign(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCampaignRequest>,
) -> ApiResult<(StatusCode, Json<CampaignDto>)> {
    authorize(&actor, "promotions.write")?;
    let campaign = PromotionService::new(db)
        .create_campaign(CreateCampaignInput {
            name: request.name,
            description: request.description,
            budget_type: request.budget_type,
            currency_code: request.currency_code,
            budget_limit: request.budget_limit,
            starts_at: request.starts_at,
            ends_at: request.ends_at,
            metadata: request.metadata,
        })
        .await?;
    record_store_audit(
        &context,
        &actor,
        "campaign.created",
        "campaign",
        &campaign.id,
        serde_json::json!({"name": campaign.name, "budget_limit": campaign.budget_limit}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(CampaignDto::from(campaign))))
}

async fn update_campaign(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCampaignRequest>,
) -> ApiResult<Json<CampaignDto>> {
    authorize(&actor, "promotions.write")?;
    let campaign = PromotionService::new(db)
        .update_campaign(
            &id,
            UpdateCampaignInput {
                name: request.name,
                description: request.description,
                budget_type: request.budget_type,
                currency_code: request.currency_code,
                budget_limit: request.budget_limit,
                starts_at: request.starts_at,
                ends_at: request.ends_at,
                metadata: request.metadata,
            },
        )
        .await?;
    record_store_audit(
        &context,
        &actor,
        "campaign.updated",
        "campaign",
        &campaign.id,
        serde_json::json!({"name": campaign.name, "budget_limit": campaign.budget_limit}),
    )
    .await;
    Ok(Json(CampaignDto::from(campaign)))
}

async fn delete_campaign(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "promotions.write")?;
    PromotionService::new(db).delete_campaign(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "campaign.deleted",
        "campaign",
        &id,
        serde_json::Value::Null,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_promotions(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<String>>> {
    authorize(&actor, "promotions.read")?;
    Ok(Json(
        PromotionService::new(db)
            .list_campaign_promotions(&id)
            .await?,
    ))
}

async fn link_promotion(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<LinkPromotionRequest>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "promotions.write")?;
    PromotionService::new(db)
        .link_promotion(&id, &request.promotion_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "campaign.promotion_linked",
        "campaign",
        &id,
        serde_json::json!({"promotion_id": request.promotion_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlink_promotion(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, promotion_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "promotions.write")?;
    PromotionService::new(db)
        .unlink_promotion(&id, &promotion_id)
        .await?;
    record_store_audit(
        &context,
        &actor,
        "campaign.promotion_unlinked",
        "campaign",
        &id,
        serde_json::json!({"promotion_id": promotion_id}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
