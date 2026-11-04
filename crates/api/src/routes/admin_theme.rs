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
        .route("/themes/{id}/activate", post(activate_theme))
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
}

impl From<RegisterThemeRequest> for RegisterThemeInput {
    fn from(request: RegisterThemeRequest) -> Self {
        Self {
            manifest: request.manifest.into(),
            bundle_path: request.bundle_path,
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

async fn register_theme(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<RegisterThemeRequest>,
) -> ApiResult<(StatusCode, Json<ThemeDto>)> {
    authorize(&actor, "themes.write")?;
    let theme = ThemeService::new(db).register(input.into()).await?;
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
        }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(ThemeDto::from(theme))))
}

async fn get_theme(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.read")?;
    let theme = ThemeService::new(db).get(&ThemeId::from_string(id)).await?;
    Ok(Json(ThemeDto::from(theme)))
}

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
