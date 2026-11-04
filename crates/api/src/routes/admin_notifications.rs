//! Admin notification channel routes.
//!
//! Channels are store-scoped integrations. Endpoint URLs and signing secrets
//! are never returned; operators see a masked URL and a configured flag.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{ActorContext, AppError, NotificationChannelId};
use fastcart_notifications::{
    ChannelDelivery, ChannelKind, ChannelTestResult, CreateChannelInput, NotificationChannel,
    NotificationChannelService, RenderContext, UpdateChannelInput, supported_channel_events,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the admin notification routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route(
            "/notification-channels",
            get(list_channels).post(create_channel),
        )
        .route(
            "/notification-channels/{id}",
            get(get_channel)
                .patch(update_channel)
                .delete(delete_channel),
        )
        .route("/notification-channels/{id}/test", post(test_channel))
        .route(
            "/notification-channels/{id}/deliveries",
            get(list_deliveries),
        )
        .route("/notification-events", get(list_events))
}

/// Builds the documented admin notification routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_channels, create_channel))
        .routes(routes!(get_channel, update_channel, delete_channel))
        .routes(routes!(test_channel))
        .routes(routes!(list_deliveries))
        .routes(routes!(list_events))
}

/// Channel response without secrets.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct NotificationChannelDto {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub config: Value,
    pub endpoint_url_masked: String,
    pub signing_secret_configured: bool,
    pub last_error: Option<String>,
    pub last_success_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Create request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateNotificationChannelRequest {
    pub kind: String,
    pub name: String,
    pub endpoint_url: String,
    #[serde(default)]
    pub signing_secret: Option<String>,
    pub events: Vec<String>,
    #[serde(default = "default_true")]
    pub is_active: bool,
    #[serde(default)]
    pub config: Value,
}

/// Update request. An empty `signing_secret` clears the stored secret.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateNotificationChannelRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub signing_secret: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub config: Option<Value>,
}

/// Delivery response.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct NotificationDeliveryDto {
    pub id: String,
    pub channel_id: String,
    pub event_id: String,
    pub event_type: String,
    pub status: String,
    pub job_id: Option<String>,
    pub attempts: i64,
    pub response_status: Option<i64>,
    pub response_body: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub sent_at: Option<i64>,
}

/// Event catalog entry.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct NotificationEventDto {
    #[serde(rename = "type")]
    pub event_type: String,
    pub label: String,
    pub group: String,
    pub description: String,
}

/// Test result.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct TestChannelResponse {
    pub ok: bool,
    pub status: Option<i64>,
    pub error: Option<String>,
}

/// Delivery list query.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeliveryListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

fn default_true() -> bool {
    true
}

fn require_permission(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

fn parse_kind(value: &str) -> ApiResult<ChannelKind> {
    match value {
        "feishu" => Ok(ChannelKind::Feishu),
        other => Err(ApiError::new(AppError::validation(format!(
            "unsupported notification channel kind: {other}"
        )))),
    }
}

fn channel_dto(channel: &NotificationChannel) -> NotificationChannelDto {
    NotificationChannelDto {
        id: channel.id.to_string(),
        kind: channel.kind.as_str().to_owned(),
        name: channel.name.clone(),
        events: channel.events.clone(),
        is_active: channel.is_active,
        config: channel.config.clone(),
        endpoint_url_masked: channel.endpoint_url_masked.clone(),
        signing_secret_configured: channel.signing_secret_configured,
        last_error: channel.last_error.clone(),
        last_success_at: channel.last_success_at,
        created_at: channel.created_at,
        updated_at: channel.updated_at,
    }
}

fn delivery_dto(delivery: &ChannelDelivery) -> NotificationDeliveryDto {
    NotificationDeliveryDto {
        id: delivery.id.to_string(),
        channel_id: delivery.channel_id.to_string(),
        event_id: delivery.event_id.to_string(),
        event_type: delivery.event_type.clone(),
        status: delivery.status.as_str().to_owned(),
        job_id: delivery.job_id.as_ref().map(ToString::to_string),
        attempts: delivery.attempts,
        response_status: delivery.response_status,
        response_body: delivery.response_body.clone(),
        last_error: delivery.last_error.clone(),
        created_at: delivery.created_at,
        updated_at: delivery.updated_at,
        sent_at: delivery.sent_at,
    }
}

fn render_context(context: &AppContext) -> RenderContext {
    RenderContext {
        admin_base_url: context.config.admin_base_url.clone(),
        locale: "zh-CN".to_owned(),
    }
}

#[utoipa::path(
    get,
    path = "/notification-channels",
    tag = "Admin",
    summary = "List notification channels",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<NotificationChannelDto>))
)]
async fn list_channels(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<Vec<NotificationChannelDto>>> {
    require_permission(&actor, "settings.read")?;
    let channels = NotificationChannelService::new().list_channels(&db).await?;
    Ok(Json(channels.iter().map(channel_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/notification-channels",
    tag = "Admin",
    summary = "Create a notification channel",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateNotificationChannelRequest,
    responses((status = 201, body = NotificationChannelDto))
)]
async fn create_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateNotificationChannelRequest>,
) -> ApiResult<(StatusCode, Json<NotificationChannelDto>)> {
    require_permission(&actor, "settings.write")?;
    let kind = parse_kind(&request.kind)?;
    let config = if request.config.is_null() {
        Value::Object(Default::default())
    } else {
        request.config
    };
    let channel = NotificationChannelService::new()
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_channel(
            &db,
            CreateChannelInput {
                kind,
                name: request.name,
                endpoint_url: request.endpoint_url,
                signing_secret: request.signing_secret,
                events: request.events,
                is_active: request.is_active,
                config,
            },
        )
        .await?;
    Ok((StatusCode::CREATED, Json(channel_dto(&channel))))
}

#[utoipa::path(
    get,
    path = "/notification-channels/{id}",
    tag = "Admin",
    summary = "Get a notification channel",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = NotificationChannelDto))
)]
async fn get_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<NotificationChannelDto>> {
    require_permission(&actor, "settings.read")?;
    let channel = NotificationChannelService::new()
        .get_channel(&db, &NotificationChannelId::from_string(id))
        .await?;
    Ok(Json(channel_dto(&channel)))
}

#[utoipa::path(
    patch,
    path = "/notification-channels/{id}",
    tag = "Admin",
    summary = "Update a notification channel",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateNotificationChannelRequest,
    responses((status = 200, body = NotificationChannelDto))
)]
async fn update_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateNotificationChannelRequest>,
) -> ApiResult<Json<NotificationChannelDto>> {
    require_permission(&actor, "settings.write")?;
    let signing_secret = request.signing_secret.map(|secret| {
        let secret = secret.trim().to_owned();
        if secret.is_empty() {
            None
        } else {
            Some(secret)
        }
    });
    let channel = NotificationChannelService::new()
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .update_channel(
            &db,
            &NotificationChannelId::from_string(id),
            UpdateChannelInput {
                name: request.name,
                endpoint_url: request.endpoint_url,
                signing_secret,
                events: request.events,
                is_active: request.is_active,
                config: request.config,
            },
        )
        .await?;
    Ok(Json(channel_dto(&channel)))
}

#[utoipa::path(
    delete,
    path = "/notification-channels/{id}",
    tag = "Admin",
    summary = "Delete a notification channel",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, "settings.write")?;
    let channel_id = NotificationChannelId::from_string(id);
    NotificationChannelService::new()
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .delete_channel(&db, &channel_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/notification-channels/{id}/test",
    tag = "Admin",
    summary = "Send a test message",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = TestChannelResponse))
)]
async fn test_channel(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<TestChannelResponse>> {
    require_permission(&actor, "settings.write")?;
    let channel_id = NotificationChannelId::from_string(id);
    let result: ChannelTestResult = NotificationChannelService::new()
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .test_channel(&db, &channel_id, &render_context(&context))
        .await?;
    Ok(Json(TestChannelResponse {
        ok: result.ok,
        status: result.status,
        error: result.error,
    }))
}

#[utoipa::path(
    get,
    path = "/notification-channels/{id}/deliveries",
    tag = "Admin",
    summary = "List notification deliveries",
    security(("adminSession" = []), ("apiToken" = [])),
    params(DeliveryListQuery),
    responses((status = 200, body = Vec<NotificationDeliveryDto>))
)]
async fn list_deliveries(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<DeliveryListQuery>,
) -> ApiResult<Json<Vec<NotificationDeliveryDto>>> {
    require_permission(&actor, "settings.read")?;
    let deliveries = NotificationChannelService::new()
        .list_deliveries(
            &db,
            &NotificationChannelId::from_string(id),
            query.limit.unwrap_or(50),
        )
        .await?;
    Ok(Json(deliveries.iter().map(delivery_dto).collect()))
}

#[utoipa::path(
    get,
    path = "/notification-events",
    tag = "Admin",
    summary = "List supported notification events",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<NotificationEventDto>))
)]
async fn list_events(AdminActor(actor): AdminActor) -> ApiResult<Json<Vec<NotificationEventDto>>> {
    require_permission(&actor, "settings.read")?;
    Ok(Json(
        supported_channel_events()
            .iter()
            .map(
                |(event_type, label, group, description)| NotificationEventDto {
                    event_type: (*event_type).to_owned(),
                    label: (*label).to_owned(),
                    group: (*group).to_owned(),
                    description: (*description).to_owned(),
                },
            )
            .collect(),
    ))
}
