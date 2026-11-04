//! Admin theme registry HTTP routes.
//!
//! Handlers enforce `themes.*` permissions and delegate all business rules to
//! [`ThemeService`].

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, AppError, ThemeId};
use fastcart_module_theme::{RegisterThemeInput, ThemeService, UpdateThemeSettingsInput};
use serde::Deserialize;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, record_store_audit};
use crate::dto::{PageResponse, ThemeDto, ThemeManifestDto};
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/themes", get(list_themes).post(register_theme))
        .route("/themes/active", get(get_active_theme))
        .route("/themes/rollback", post(rollback_theme))
        .route("/themes/{id}", get(get_theme).delete(delete_theme))
        .route("/themes/{id}/settings", patch(update_theme_settings))
        .route(
            "/themes/{id}/editor-manifest",
            get(get_theme_editor_manifest),
        )
        .route("/themes/{id}/activate", post(activate_theme))
}

/// Builds the documented admin theme routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_themes, register_theme))
        .routes(routes!(get_active_theme))
        .routes(routes!(rollback_theme))
        .routes(routes!(get_theme, delete_theme))
        .routes(routes!(update_theme_settings))
        .routes(routes!(get_theme_editor_manifest))
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
    bundle_path: String,
    #[serde(default)]
    settings_schema: Option<Value>,
    #[serde(default)]
    settings: Option<Value>,
    /// Raw editor manifest; required when `manifest.editor` is present.
    #[serde(default)]
    editor_manifest: Option<Value>,
}

impl From<RegisterThemeRequest> for RegisterThemeInput {
    fn from(request: RegisterThemeRequest) -> Self {
        Self {
            manifest: request.manifest.into(),
            bundle_path: request.bundle_path,
            settings_schema: request.settings_schema,
            settings: request.settings,
            editor_manifest: request.editor_manifest,
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<RegisterThemeRequest>,
) -> ApiResult<(StatusCode, Json<ThemeDto>)> {
    authorize(&actor, "themes.write")?;
    let mut input: RegisterThemeInput = input.into();
    // Prefer an explicit manifest (compatibility and tests), otherwise load
    // the trusted bundle's `editor.manifest.json` so operators do not have to
    // paste it into the request.
    if input.editor_manifest.is_none()
        && let Some(editor) = input.manifest.editor.as_ref()
    {
        let loaded = fastcart_module_theme::load_editor_manifest(
            &context.config.themes_root,
            &input.bundle_path,
            &editor.manifest,
        )
        .await?;
        input.editor_manifest = Some(serde_json::to_value(&loaded).map_err(|error| {
            ApiError::new(AppError::internal(format!(
                "could not serialize editor manifest: {error}"
            )))
        })?);
    }
    let theme = ThemeService::new(db).register(input).await?;
    record_store_audit(
        &context,
        &actor,
        "theme.registered",
        "theme",
        theme.id.as_str(),
        serde_json::json!({
            "manifest_id": theme.manifest.id.clone(),
            "version": theme.manifest.version.clone(),
            "bundle_path": theme.bundle_path.clone(),
            "has_editor_manifest": theme.editor_manifest.is_some(),
        }),
    )
    .await;
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
    path = "/themes/{id}/editor-manifest",
    tag = "Admin",
    summary = "Get a theme editor manifest",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn get_theme_editor_manifest(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    authorize(&actor, "themes.read")?;
    let manifest = ThemeService::new(db)
        .editor_manifest(&ThemeId::from_string(id))
        .await?;
    let manifest = serde_json::to_value(manifest)
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;
    Ok(Json(manifest))
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<UpdateThemeSettingsRequest>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .update_settings(&ThemeId::from_string(id), input.into())
        .await?;
    record_store_audit(
        &context,
        &actor,
        "theme.settings_updated",
        "theme",
        theme.id.as_str(),
        serde_json::json!({"manifest_id": theme.manifest.id.clone()}),
    )
    .await;
    Ok(Json(ThemeDto::from(theme)))
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db)
        .activate(&ThemeId::from_string(id))
        .await?;
    record_store_audit(
        &context,
        &actor,
        "theme.activated",
        "theme",
        theme.id.as_str(),
        serde_json::json!({"manifest_id": theme.manifest.id.clone()}),
    )
    .await;
    Ok(Json(ThemeDto::from(theme)))
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db).rollback().await?;
    record_store_audit(
        &context,
        &actor,
        "theme.rolled_back",
        "theme",
        theme.id.as_str(),
        serde_json::json!({"manifest_id": theme.manifest.id.clone()}),
    )
    .await;
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let theme_id = ThemeId::from_string(id);
    ThemeService::new(db).delete(&theme_id).await?;
    record_store_audit(
        &context,
        &actor,
        "theme.deleted",
        "theme",
        theme_id.as_str(),
        serde_json::Value::Null,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
