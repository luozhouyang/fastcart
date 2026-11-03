//! Admin webhook endpoint and delivery routes.
//!
//! These endpoints require `settings.read` to inspect subscriptions and
//! deliveries and `settings.write` to change them. Signing secrets are returned
//! only when an endpoint is created or rotated.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{get, post};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{ActorContext, Metadata, WebhookDeliveryId, WebhookId};
use fastcart_module_webhook::{
    CreateWebhookEndpointInput, UpdateWebhookEndpointInput, WebhookDelivery, WebhookEndpoint,
    WebhookService,
};

use super::StoreDb;

/// Builds the admin webhook routes.
pub fn router() -> Router<Arc<crate::state::AppContext>> {
    Router::new()
        .route("/webhooks", get(list_endpoints).post(create_endpoint))
        .route(
            "/webhooks/{id}",
            get(get_endpoint)
                .patch(update_endpoint)
                .delete(delete_endpoint),
        )
        .route("/webhooks/{id}/rotate-secret", post(rotate_secret))
        .route("/webhooks/{id}/deliveries", get(list_deliveries))
        .route("/webhook-deliveries/{id}/replay", post(replay_delivery))
}

#[derive(Clone, Debug, serde::Deserialize)]
struct CreateWebhookRequest {
    url: String,
    events: Vec<String>,
    secret: Option<String>,
    metadata: Option<Metadata>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct UpdateWebhookRequest {
    url: Option<String>,
    events: Option<Vec<String>>,
    is_active: Option<bool>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct DeliveriesQuery {
    limit: Option<i64>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebhookEndpointDto {
    id: String,
    url: String,
    events: Vec<String>,
    is_active: bool,
    metadata: Metadata,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CreatedWebhookDto {
    #[serde(flatten)]
    endpoint: WebhookEndpointDto,
    secret: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct RotatedSecretDto {
    secret: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct WebhookDeliveryDto {
    id: String,
    endpoint_id: String,
    event_id: String,
    event_type: String,
    payload: serde_json::Value,
    status: String,
    attempts: i64,
    response_status: Option<i64>,
    response_body: Option<String>,
    next_retry_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

fn endpoint_dto(endpoint: &WebhookEndpoint) -> WebhookEndpointDto {
    WebhookEndpointDto {
        id: endpoint.id.to_string(),
        url: endpoint.url.clone(),
        events: endpoint.events.clone(),
        is_active: endpoint.is_active,
        metadata: endpoint.metadata.clone(),
        created_at: endpoint.created_at,
        updated_at: endpoint.updated_at,
    }
}

fn delivery_dto(delivery: &WebhookDelivery) -> WebhookDeliveryDto {
    WebhookDeliveryDto {
        id: delivery.id.to_string(),
        endpoint_id: delivery.endpoint_id.to_string(),
        event_id: delivery.event_id.clone(),
        event_type: delivery.event_type.clone(),
        payload: delivery.payload.clone(),
        status: delivery.status.as_str().to_owned(),
        attempts: delivery.attempts,
        response_status: delivery.response_status,
        response_body: delivery.response_body.clone(),
        next_retry_at: delivery.next_retry_at,
        created_at: delivery.created_at,
        updated_at: delivery.updated_at,
    }
}

async fn list_endpoints(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<Vec<WebhookEndpointDto>>> {
    require_permission(&actor, "settings.read")?;
    let endpoints = WebhookService::new(db).list_endpoints().await?;
    Ok(Json(endpoints.iter().map(endpoint_dto).collect()))
}

async fn create_endpoint(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateWebhookRequest>,
) -> ApiResult<(StatusCode, Json<CreatedWebhookDto>)> {
    require_permission(&actor, "settings.write")?;
    let created = WebhookService::new(db)
        .create_endpoint(CreateWebhookEndpointInput {
            url: request.url,
            events: request.events,
            secret: request.secret,
            metadata: request.metadata,
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedWebhookDto {
            endpoint: endpoint_dto(&created.endpoint),
            secret: created.secret,
        }),
    ))
}

async fn get_endpoint(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<WebhookEndpointDto>> {
    require_permission(&actor, "settings.read")?;
    let endpoint = WebhookService::new(db)
        .get_endpoint(&WebhookId::from_string(id))
        .await?;
    Ok(Json(endpoint_dto(&endpoint)))
}

async fn update_endpoint(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateWebhookRequest>,
) -> ApiResult<Json<WebhookEndpointDto>> {
    require_permission(&actor, "settings.write")?;
    let endpoint = WebhookService::new(db)
        .update_endpoint(
            &WebhookId::from_string(id),
            UpdateWebhookEndpointInput {
                url: request.url,
                events: request.events,
                is_active: request.is_active,
            },
        )
        .await?;
    Ok(Json(endpoint_dto(&endpoint)))
}

async fn delete_endpoint(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, "settings.write")?;
    WebhookService::new(db)
        .delete_endpoint(&WebhookId::from_string(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rotate_secret(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<RotatedSecretDto>> {
    require_permission(&actor, "settings.write")?;
    let secret = WebhookService::new(db)
        .rotate_secret(&WebhookId::from_string(id))
        .await?;
    Ok(Json(RotatedSecretDto { secret }))
}

async fn list_deliveries(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<DeliveriesQuery>,
) -> ApiResult<Json<Vec<WebhookDeliveryDto>>> {
    require_permission(&actor, "settings.read")?;
    let deliveries = WebhookService::new(db)
        .list_deliveries(&WebhookId::from_string(id), query.limit.unwrap_or(50))
        .await?;
    Ok(Json(deliveries.iter().map(delivery_dto).collect()))
}

async fn replay_delivery(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<WebhookDeliveryDto>> {
    require_permission(&actor, "settings.write")?;
    let delivery = WebhookService::new(db)
        .replay(&WebhookDeliveryId::from_string(id))
        .await?;
    Ok(Json(delivery_dto(&delivery)))
}
