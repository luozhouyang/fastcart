//! Admin regions and storefront publishable API keys.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, Metadata};
use fastcart_module_store::{
    CreatePublishableApiKeyInput, CreateRegionInput, CreatedPublishableApiKey, PublishableApiKey,
    Region, RegionService, UpdateRegionInput,
};
use serde::{Deserialize, Serialize};

use super::{StoreDb, record_store_audit};
use crate::dto::{PageInfo, PageResponse};
use crate::state::AppContext;

/// Builds the region and publishable-key routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/regions", get(list_regions).post(create_region))
        .route(
            "/regions/{id}",
            get(get_region).patch(update_region).delete(delete_region),
        )
        .route("/publishable-api-keys", get(list_keys).post(create_key))
        .route("/publishable-api-keys/{id}", delete(revoke_key))
}

/// Region response body.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RegionDto {
    /// Region identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Currency code.
    pub currency_code: String,
    /// Country codes.
    pub countries: Vec<String>,
    /// Tax region.
    pub tax_region_id: Option<String>,
    /// Enabled payment providers.
    pub payment_providers: Vec<String>,
    /// Whether this is the default region.
    pub is_default: bool,
    /// Extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp.
    pub created_at: i64,
    /// Last update timestamp.
    pub updated_at: i64,
}

impl From<Region> for RegionDto {
    fn from(region: Region) -> Self {
        Self {
            id: region.id,
            name: region.name,
            currency_code: region.currency_code,
            countries: region.countries,
            tax_region_id: region.tax_region_id,
            payment_providers: region.payment_providers,
            is_default: region.is_default,
            metadata: region.metadata,
            created_at: region.created_at,
            updated_at: region.updated_at,
        }
    }
}

/// Region creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRegionRequest {
    /// Display name.
    pub name: String,
    /// ISO currency code.
    pub currency_code: String,
    /// ISO country codes.
    pub countries: Vec<String>,
    /// Optional tax region.
    pub tax_region_id: Option<String>,
    /// Enabled payment providers.
    #[serde(default)]
    pub payment_providers: Vec<String>,
    /// Whether this is the default region.
    #[serde(default)]
    pub is_default: bool,
    /// Extension metadata.
    #[serde(default)]
    pub metadata: Metadata,
}

/// Region update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateRegionRequest {
    /// New name.
    pub name: Option<String>,
    /// New currency.
    pub currency_code: Option<String>,
    /// Replacement countries.
    pub countries: Option<Vec<String>>,
    /// Replacement tax region; `null` clears it.
    pub tax_region_id: Option<Option<String>>,
    /// Replacement payment providers.
    pub payment_providers: Option<Vec<String>>,
    /// Whether this is the default region.
    pub is_default: Option<bool>,
    /// Replacement metadata.
    pub metadata: Option<Metadata>,
}

/// Publishable key response body.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PublishableKeyDto {
    /// Key identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Display prefix.
    pub token_prefix: String,
    /// Allowed sales channels.
    pub sales_channel_ids: Vec<String>,
    /// Revocation timestamp.
    pub revoked_at: Option<i64>,
    /// Last use timestamp.
    pub last_used_at: Option<i64>,
    /// Creation timestamp.
    pub created_at: i64,
}

impl From<PublishableApiKey> for PublishableKeyDto {
    fn from(key: PublishableApiKey) -> Self {
        Self {
            id: key.id,
            title: key.title,
            token_prefix: key.token_prefix,
            sales_channel_ids: key.sales_channel_ids,
            revoked_at: key.revoked_at,
            last_used_at: key.last_used_at,
            created_at: key.created_at,
        }
    }
}

/// Created publishable key body; the secret is shown once.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreatedPublishableKeyDto {
    /// Persisted metadata.
    #[serde(flatten)]
    pub key: PublishableKeyDto,
    /// One-time `pk_...` secret.
    pub token: String,
}

/// Publishable key creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePublishableKeyRequest {
    /// Human-readable title.
    pub title: String,
    /// Sales channels the key may access.
    pub sales_channel_ids: Vec<String>,
}

fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

async fn list_regions(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<RegionDto>>> {
    authorize(&actor, "settings.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = RegionService::new(db).list_regions(pagination).await?;
    let page = page.map(RegionDto::from);
    Ok(Json(PageResponse {
        data: page.data,
        pagination: PageInfo {
            limit: page.pagination.limit,
            offset: page.pagination.offset,
            total: page.count,
        },
    }))
}

async fn get_region(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<RegionDto>> {
    authorize(&actor, "settings.read")?;
    Ok(Json(RegionDto::from(
        RegionService::new(db).get_region(&id).await?,
    )))
}

async fn create_region(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateRegionRequest>,
) -> ApiResult<(StatusCode, Json<RegionDto>)> {
    authorize(&actor, "settings.write")?;
    let region = RegionService::new(db)
        .create_region(CreateRegionInput {
            name: request.name,
            currency_code: request.currency_code,
            countries: request.countries,
            tax_region_id: request.tax_region_id,
            payment_providers: request.payment_providers,
            is_default: request.is_default,
            metadata: request.metadata,
        })
        .await?;
    record_store_audit(
        &context,
        &actor,
        "region.created",
        "region",
        &region.id,
        serde_json::json!({"name": region.name, "is_default": region.is_default}),
    )
    .await;
    Ok((StatusCode::CREATED, Json(RegionDto::from(region))))
}

async fn update_region(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateRegionRequest>,
) -> ApiResult<Json<RegionDto>> {
    authorize(&actor, "settings.write")?;
    let region = RegionService::new(db)
        .update_region(
            &id,
            UpdateRegionInput {
                name: request.name,
                currency_code: request.currency_code,
                countries: request.countries,
                tax_region_id: request.tax_region_id,
                payment_providers: request.payment_providers,
                is_default: request.is_default,
                metadata: request.metadata,
            },
        )
        .await?;
    record_store_audit(
        &context,
        &actor,
        "region.updated",
        "region",
        &region.id,
        serde_json::json!({"name": region.name, "is_default": region.is_default}),
    )
    .await;
    Ok(Json(RegionDto::from(region)))
}

async fn delete_region(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "settings.write")?;
    RegionService::new(db).delete_region(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "region.deleted",
        "region",
        &id,
        serde_json::Value::Null,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_keys(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<Vec<PublishableKeyDto>>> {
    authorize(&actor, "settings.read")?;
    let keys = RegionService::new(db).list_publishable_keys().await?;
    Ok(Json(
        keys.into_iter().map(PublishableKeyDto::from).collect(),
    ))
}

async fn create_key(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreatePublishableKeyRequest>,
) -> ApiResult<(StatusCode, Json<CreatedPublishableKeyDto>)> {
    authorize(&actor, "settings.write")?;
    let created: CreatedPublishableApiKey = RegionService::new(db)
        .create_publishable_key(CreatePublishableApiKeyInput {
            title: request.title,
            sales_channel_ids: request.sales_channel_ids,
        })
        .await?;
    record_store_audit(
        &context,
        &actor,
        "publishable_key.created",
        "publishable_api_key",
        &created.key.id,
        serde_json::json!({"sales_channel_ids": created.key.sales_channel_ids}),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedPublishableKeyDto {
            token: created.token,
            key: PublishableKeyDto::from(created.key),
        }),
    ))
}

async fn revoke_key(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "settings.write")?;
    RegionService::new(db).revoke_publishable_key(&id).await?;
    record_store_audit(
        &context,
        &actor,
        "publishable_key.revoked",
        "publishable_api_key",
        &id,
        serde_json::Value::Null,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
