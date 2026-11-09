//! Admin theme registry HTTP routes.
//!
//! Handlers enforce `themes.*` permissions and delegate all business rules to
//! [`ThemeService`]. Themes are deployment artifacts (ADR 0008): registration
//! stores metadata, an optional settings schema and the initial settings; the
//! registry never receives bundle contents.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, AppError, Pagination, ThemeId};
use fastcart_module_theme::{
    RegisterThemeInput, Theme, ThemeActivationRecord, ThemeService, ThemeSettingsRevision,
    UpdateThemeSettingsInput,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{PageResponse, ThemeDto, ThemeManifestDto};
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/themes", get(list_themes).post(register_theme))
        .route("/themes/active", get(get_active_theme))
        .route("/themes/activations", get(list_theme_activations))
        .route("/themes/rollback", post(rollback_theme))
        .route("/themes/{id}", get(get_theme).delete(delete_theme))
        .route("/themes/{id}/settings", patch(update_theme_settings))
        .route(
            "/themes/{id}/settings/draft",
            get(get_theme_settings_draft).put(save_theme_settings_draft),
        )
        .route(
            "/themes/{id}/settings/publish",
            post(publish_theme_settings),
        )
        .route(
            "/themes/{id}/settings/revisions",
            get(list_theme_settings_revisions),
        )
        .route(
            "/themes/{id}/settings/revisions/{revision}/restore",
            post(restore_theme_settings_revision),
        )
        .route("/themes/{id}/activate", post(activate_theme))
}

/// Builds the documented admin theme routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_themes, register_theme))
        .routes(routes!(get_active_theme))
        .routes(routes!(list_theme_activations))
        .routes(routes!(rollback_theme))
        .routes(routes!(get_theme, delete_theme))
        .routes(routes!(update_theme_settings))
        .routes(routes!(get_theme_settings_draft, save_theme_settings_draft))
        .routes(routes!(publish_theme_settings))
        .routes(routes!(list_theme_settings_revisions))
        .routes(routes!(restore_theme_settings_revision))
        .routes(routes!(activate_theme))
}

/// Enforces a permission at the handler boundary.
fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

/// Theme registration body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct RegisterThemeRequest {
    manifest: ThemeManifestDto,
    #[serde(default)]
    settings_schema: Option<Value>,
    #[serde(default)]
    settings: Option<Value>,
}

impl From<RegisterThemeRequest> for RegisterThemeInput {
    fn from(request: RegisterThemeRequest) -> Self {
        Self {
            manifest: request.manifest.into(),
            settings_schema: request.settings_schema,
            settings: request.settings,
        }
    }
}

/// Theme settings replacement body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateThemeSettingsRequest {
    settings: Value,
}

impl From<UpdateThemeSettingsRequest> for UpdateThemeSettingsInput {
    fn from(request: UpdateThemeSettingsRequest) -> Self {
        Self {
            settings: request.settings,
        }
    }
}

/// Settings draft body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct SaveThemeSettingsDraftRequest {
    settings: Value,
    /// Revision the caller last observed, enabling optimistic concurrency.
    #[serde(default)]
    base_revision: Option<i64>,
}

/// Optional concurrency token for publish and restore.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct ThemeSettingsRevisionRequest {
    /// Revision the caller last observed, enabling optimistic concurrency.
    #[serde(default)]
    base_revision: Option<i64>,
}

/// Settings state exposed by the draft and revision endpoints.
#[derive(Clone, Debug, Serialize)]
pub struct ThemeSettingsStateResponse {
    theme_id: String,
    settings: Value,
    draft_settings: Option<Value>,
    settings_revision: i64,
}

impl From<Theme> for ThemeSettingsStateResponse {
    fn from(theme: Theme) -> Self {
        Self {
            theme_id: theme.id.to_string(),
            settings: theme.settings,
            draft_settings: theme.draft_settings,
            settings_revision: theme.settings_revision,
        }
    }
}

/// One settings revision exposed to administrators.
#[derive(Clone, Debug, Serialize)]
pub struct ThemeSettingsRevisionResponse {
    id: String,
    theme_id: String,
    revision: i64,
    settings: Value,
    status: String,
    created_by: Option<String>,
    created_at: i64,
}

impl From<ThemeSettingsRevision> for ThemeSettingsRevisionResponse {
    fn from(revision: ThemeSettingsRevision) -> Self {
        Self {
            id: revision.id,
            theme_id: revision.theme_id.to_string(),
            revision: revision.revision,
            settings: revision.settings,
            status: revision.status.as_str().to_owned(),
            created_by: revision.created_by,
            created_at: revision.created_at,
        }
    }
}

/// One activation history entry exposed to administrators.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ThemeActivationResponse {
    id: String,
    theme_id: String,
    manifest_id: Option<String>,
    theme_name: Option<String>,
    theme_version: Option<String>,
    previous_theme_id: Option<String>,
    created_at: i64,
}

impl From<ThemeActivationRecord> for ThemeActivationResponse {
    fn from(record: ThemeActivationRecord) -> Self {
        Self {
            id: record.id.to_string(),
            theme_id: record.theme_id.to_string(),
            manifest_id: record.manifest_id,
            theme_name: record.theme_name,
            theme_version: record.theme_version,
            previous_theme_id: record.previous_theme_id.map(|id| id.to_string()),
            created_at: record.created_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/themes",
    tag = "Admin",
    summary = "List registered themes",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<ThemeDto>))
)]
async fn list_themes(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ThemeDto>>> {
    authorize(&actor, "themes.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ThemeService::new(db).list(pagination).await?;
    Ok(Json(PageResponse::from_page(page.map(ThemeDto::from))))
}

#[utoipa::path(
    post,
    path = "/themes",
    tag = "Admin",
    summary = "Register a theme",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = RegisterThemeRequest,
    responses((status = 201, body = ThemeDto))
)]
async fn register_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<RegisterThemeRequest>,
) -> ApiResult<(StatusCode, Json<ThemeDto>)> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .register(input.into())
        .await?;
    Ok((StatusCode::CREATED, Json(ThemeDto::from(theme))))
}

#[utoipa::path(
    get,
    path = "/themes/{id}",
    tag = "Admin",
    summary = "Get a theme",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ThemeDto))
)]
async fn get_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.read")?;
    let theme = ThemeService::new(db).get(&ThemeId::from_string(id)).await?;
    Ok(Json(ThemeDto::from(theme)))
}

#[utoipa::path(
    get,
    path = "/themes/active",
    tag = "Admin",
    summary = "Get the active theme",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ThemeDto))
)]
async fn get_active_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.read")?;
    let theme = ThemeService::new(db)
        .active()
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("theme", "active")))?;
    Ok(Json(ThemeDto::from(theme)))
}

#[utoipa::path(
    patch,
    path = "/themes/{id}/settings",
    tag = "Admin",
    summary = "Replace theme settings",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateThemeSettingsRequest,
    responses((status = 200, body = ThemeDto))
)]
async fn update_theme_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<UpdateThemeSettingsRequest>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .update_settings(&ThemeId::from_string(id), input.into())
        .await?;
    Ok(Json(ThemeDto::from(theme)))
}

#[utoipa::path(
    get,
    path = "/themes/{id}/settings/draft",
    tag = "Admin",
    summary = "Get theme settings draft",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_theme_settings_draft(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ThemeSettingsStateResponse>> {
    authorize(&actor, "themes.read")?;
    let theme = ThemeService::new(db).get(&ThemeId::from_string(id)).await?;
    Ok(Json(theme.into()))
}

#[utoipa::path(
    put,
    path = "/themes/{id}/settings/draft",
    tag = "Admin",
    summary = "Save theme settings draft",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value))
)]
async fn save_theme_settings_draft(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<SaveThemeSettingsDraftRequest>,
) -> ApiResult<Json<ThemeSettingsStateResponse>> {
    authorize(&actor, "themes.write")?;
    let actor_id = actor.user_id.as_ref().map(ToString::to_string);
    let theme = ThemeService::new(db)
        .with_audit_actor(actor_id.clone())
        .save_settings_draft(
            &ThemeId::from_string(id),
            input.settings,
            actor_id,
            input.base_revision,
        )
        .await?;
    Ok(Json(theme.into()))
}

#[utoipa::path(
    post,
    path = "/themes/{id}/settings/publish",
    tag = "Admin",
    summary = "Publish theme settings draft",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn publish_theme_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ThemeSettingsRevisionRequest>,
) -> ApiResult<Json<ThemeSettingsStateResponse>> {
    authorize(&actor, "themes.write")?;
    let actor_id = actor.user_id.as_ref().map(ToString::to_string);
    let base_revision = body.base_revision;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor_id.clone())
        .publish_settings(&ThemeId::from_string(id), actor_id, base_revision)
        .await?;
    Ok(Json(theme.into()))
}

#[utoipa::path(
    get,
    path = "/themes/{id}/settings/revisions",
    tag = "Admin",
    summary = "List theme settings revisions",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = serde_json::Value))
)]
async fn list_theme_settings_revisions(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    PaginationQuery { limit, .. }: PaginationQuery,
) -> ApiResult<Json<Vec<ThemeSettingsRevisionResponse>>> {
    authorize(&actor, "themes.read")?;
    let limit = Pagination::new(limit, None).map_err(ApiError::new)?.limit;
    let revisions = ThemeService::new(db)
        .settings_revisions(&ThemeId::from_string(id), limit)
        .await?;
    Ok(Json(
        revisions
            .into_iter()
            .map(ThemeSettingsRevisionResponse::from)
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/themes/{id}/settings/revisions/{revision}/restore",
    tag = "Admin",
    summary = "Restore a theme settings revision",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn restore_theme_settings_revision(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, revision)): Path<(String, i64)>,
    ApiJson(body): ApiJson<ThemeSettingsRevisionRequest>,
) -> ApiResult<Json<ThemeSettingsStateResponse>> {
    authorize(&actor, "themes.write")?;
    let actor_id = actor.user_id.as_ref().map(ToString::to_string);
    let base_revision = body.base_revision;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor_id.clone())
        .restore_settings_revision(&ThemeId::from_string(id), revision, actor_id, base_revision)
        .await?;
    Ok(Json(theme.into()))
}

#[utoipa::path(
    post,
    path = "/themes/{id}/activate",
    tag = "Admin",
    summary = "Activate a theme",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ThemeDto))
)]
async fn activate_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .activate(&ThemeId::from_string(id))
        .await?;
    Ok(Json(ThemeDto::from(theme)))
}

#[utoipa::path(
    get,
    path = "/themes/activations",
    tag = "Admin",
    summary = "List theme activation history",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = Vec<ThemeActivationResponse>))
)]
async fn list_theme_activations(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, .. }: PaginationQuery,
) -> ApiResult<Json<Vec<ThemeActivationResponse>>> {
    authorize(&actor, "themes.read")?;
    let limit = Pagination::new(limit, None).map_err(ApiError::new)?.limit;
    let activations = ThemeService::new(db)
        .activation_history(limit)
        .await?
        .into_iter()
        .map(ThemeActivationResponse::from)
        .collect();
    Ok(Json(activations))
}

#[utoipa::path(
    post,
    path = "/themes/rollback",
    tag = "Admin",
    summary = "Roll back to the previous theme",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ThemeDto))
)]
async fn rollback_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .rollback()
        .await?;
    Ok(Json(ThemeDto::from(theme)))
}

#[utoipa::path(
    delete,
    path = "/themes/{id}",
    tag = "Admin",
    summary = "Delete a theme",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let theme_id = ThemeId::from_string(id);
    ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .delete(&theme_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
