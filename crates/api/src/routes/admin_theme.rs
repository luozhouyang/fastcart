//! Admin theme registry HTTP routes.
//!
//! Handlers enforce `themes.*` permissions and delegate all business rules to
//! [`ThemeService`].

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, patch, post};
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{ActorContext, AppError, Pagination, ThemeId};
use fastcart_module_content::ContentService;
use fastcart_module_theme::{
    RegisterThemeInput, Theme, ThemeActivationRecord, ThemeCapabilityBinding,
    ThemeCapabilityBindingInput, ThemeService, ThemeSettingsRevision, UpdateThemeSettingsInput,
};
use fastcart_storefront::ThemeEditorManifest;
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
        .route(
            "/themes/{id}/capabilities",
            get(list_theme_capabilities)
                .put(upsert_theme_capability)
                .delete(delete_theme_capability),
        )
        .route("/themes/{id}/editor.js", get(get_theme_editor_js))
        .route("/themes/{id}/validate-pages", post(validate_theme_pages))
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
        .routes(routes!(list_theme_activations))
        .routes(routes!(rollback_theme))
        .routes(routes!(get_theme, delete_theme))
        .routes(routes!(update_theme_settings))
        .routes(routes!(get_theme_settings_draft, save_theme_settings_draft))
        .routes(routes!(publish_theme_settings))
        .routes(routes!(list_theme_settings_revisions))
        .routes(routes!(restore_theme_settings_revision))
        .routes(routes!(
            list_theme_capabilities,
            upsert_theme_capability,
            delete_theme_capability
        ))
        .routes(routes!(get_theme_editor_js))
        .routes(routes!(validate_theme_pages))
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
    /// Validated assets manifest; loaded from the bundle when omitted.
    #[serde(default)]
    assets_manifest: Option<Value>,
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
            assets_manifest: request.assets_manifest,
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

/// Capability binding upsert body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ThemeCapabilityBindingRequest {
    capability_id: String,
    endpoint: String,
    #[serde(default = "default_capability_config")]
    config: Value,
    #[serde(default = "default_capability_active")]
    is_active: bool,
}

fn default_capability_config() -> Value {
    Value::Object(serde_json::Map::new())
}

const fn default_capability_active() -> bool {
    true
}

impl From<ThemeCapabilityBindingRequest> for ThemeCapabilityBindingInput {
    fn from(request: ThemeCapabilityBindingRequest) -> Self {
        Self {
            capability_id: request.capability_id,
            endpoint: request.endpoint,
            config: request.config,
            is_active: request.is_active,
        }
    }
}

/// Capability binding deletion body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct DeleteThemeCapabilityRequest {
    capability_id: String,
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

/// One capability binding exposed to administrators.
#[derive(Clone, Debug, Serialize)]
pub struct ThemeCapabilityBindingResponse {
    id: String,
    theme_id: String,
    capability_id: String,
    endpoint: String,
    config: Value,
    is_active: bool,
    created_at: i64,
    updated_at: i64,
}

impl From<ThemeCapabilityBinding> for ThemeCapabilityBindingResponse {
    fn from(binding: ThemeCapabilityBinding) -> Self {
        Self {
            id: binding.id,
            theme_id: binding.theme_id.to_string(),
            capability_id: binding.capability_id,
            endpoint: binding.endpoint,
            config: binding.config,
            is_active: binding.is_active,
            created_at: binding.created_at,
            updated_at: binding.updated_at,
        }
    }
}

/// Per-page validation issue.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct PageValidationIssue {
    page_id: String,
    handle: String,
    error: String,
}

/// Result of validating every page against a theme editor manifest.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ValidatePagesResponse {
    valid: bool,
    issues: Vec<PageValidationIssue>,
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
    // Prefer explicit manifests (compatibility and tests), otherwise load the
    // trusted bundle files so operators do not have to paste them into the
    // request. Missing files are only an error when the bundle directory
    // exists, which keeps registrations against fake bundle paths working.
    let bundle_exists = fastcart_module_theme::bundle_directory_exists(
        &context.config.themes_root,
        &input.bundle_path,
    );
    if bundle_exists
        && input.assets_manifest.is_none()
        && let Some(assets_file) = input.manifest.assets_manifest.clone()
    {
        input.assets_manifest = Some(
            fastcart_module_theme::load_assets_manifest(
                &context.config.themes_root,
                &input.bundle_path,
                &assets_file,
            )
            .await?,
        );
    }
    if bundle_exists
        && input.settings_schema.is_none()
        && let Some(schema_file) = input.manifest.settings_schema.clone()
    {
        input.settings_schema = Some(
            fastcart_module_theme::load_settings_schema(
                &context.config.themes_root,
                &input.bundle_path,
                &schema_file,
            )
            .await?,
        );
    }
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
    let theme = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .register(input)
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
    path = "/themes/{id}/editor.js",
    tag = "Admin",
    summary = "Serve a theme editor bundle",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, description = "Theme editor JavaScript bundle"))
)]
async fn get_theme_editor_js(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    authorize(&actor, "themes.read")?;
    let theme_id = ThemeId::from_string(id);
    let theme = ThemeService::new(db).get(&theme_id).await?;
    let editor = theme
        .manifest
        .editor
        .ok_or_else(|| ApiError::new(AppError::not_found("theme editor", theme_id.as_str())))?;
    // Only the manifest-declared editor entry is readable; request paths never
    // reach the filesystem.
    let bytes = fastcart_module_theme::load_theme_bundle_file(
        &context.config.themes_root,
        &theme.bundle_path,
        &editor.entry,
    )
    .await?;
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(bytes))
        .map_err(|error| {
            ApiError::new(AppError::internal(format!(
                "could not build theme editor response: {error}"
            )))
        })
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
    get,
    path = "/themes/{id}/capabilities",
    tag = "Admin",
    summary = "List theme capability bindings",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn list_theme_capabilities(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ThemeCapabilityBindingResponse>>> {
    authorize(&actor, "themes.read")?;
    let bindings = ThemeService::new(db)
        .list_capability_bindings(&ThemeId::from_string(id))
        .await?;
    Ok(Json(
        bindings
            .into_iter()
            .map(ThemeCapabilityBindingResponse::from)
            .collect(),
    ))
}

#[utoipa::path(
    put,
    path = "/themes/{id}/capabilities",
    tag = "Admin",
    summary = "Create or replace a theme capability binding",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = serde_json::Value,
    responses((status = 200, body = serde_json::Value))
)]
async fn upsert_theme_capability(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<ThemeCapabilityBindingRequest>,
) -> ApiResult<Json<ThemeCapabilityBindingResponse>> {
    authorize(&actor, "themes.write")?;
    let binding = ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .upsert_capability_binding(&ThemeId::from_string(id), input.into())
        .await?;
    Ok(Json(binding.into()))
}

#[utoipa::path(
    delete,
    path = "/themes/{id}/capabilities",
    tag = "Admin",
    summary = "Delete a theme capability binding",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = serde_json::Value,
    responses((status = 204))
)]
async fn delete_theme_capability(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<DeleteThemeCapabilityRequest>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    ThemeService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .delete_capability_binding(&ThemeId::from_string(id), &input.capability_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/themes/{id}/validate-pages",
    tag = "Admin",
    summary = "Validate store pages against a theme editor manifest",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = serde_json::Value))
)]
async fn validate_theme_pages(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ValidatePagesResponse>> {
    authorize(&actor, "themes.read")?;
    let theme_id = ThemeId::from_string(id);
    let service = ThemeService::new(db.clone());
    let editor = service.editor_manifest(&theme_id).await?;

    let issues = ContentService::new(db)
        .validate_all_pages(&editor)
        .await?
        .into_iter()
        .map(|issue| PageValidationIssue {
            page_id: issue.page_id.to_string(),
            handle: issue.handle,
            error: issue.error,
        })
        .collect::<Vec<_>>();

    Ok(Json(ValidatePagesResponse {
        valid: issues.is_empty(),
        issues,
    }))
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
    let service = ThemeService::new(db.clone())
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    let theme_id = ThemeId::from_string(id);
    let target = service.get(&theme_id).await?;
    ensure_theme_activatable(&context, &db, &target).await?;
    let theme = service.activate(&theme_id).await?;
    Ok(Json(ThemeDto::from(theme)))
}

/// Refuses to make `target` the active theme when doing so would break the
/// storefront: its runtime must match the configured runtime, every required
/// capability must be bound, and every page must render under its editor.
async fn ensure_theme_activatable(
    context: &AppContext,
    db: &fastcart_infrastructure::StoreDatabase,
    target: &Theme,
) -> ApiResult<()> {
    let configured_runtime = context.config.storefront_runtime_kind.as_str();
    if configured_runtime != "auto" && configured_runtime != target.manifest.runtime.as_str() {
        return Err(ApiError::new(AppError::conflict(format!(
            "theme runtime {} is incompatible with the configured storefront runtime \
             {configured_runtime}",
            target.manifest.runtime
        ))));
    }
    let service = ThemeService::new(db.clone());
    // Refuse activation when a required capability has no active binding.
    if let Some(requires) = target.manifest.requires.as_ref() {
        let bindings = service.list_capability_bindings(&target.id).await?;
        let missing: Vec<String> = requires
            .capabilities
            .iter()
            .filter(|capability| !capability.optional)
            .filter(|capability| {
                !bindings.iter().any(|binding| {
                    binding.is_active && binding.capability_id.as_str() == capability.id.as_str()
                })
            })
            .map(|capability| capability.id.clone())
            .collect();
        if !missing.is_empty() {
            return Err(ApiError::new(AppError::conflict_details(
                format!(
                    "theme cannot be activated because required capabilities are not bound: {}",
                    missing.join(", ")
                ),
                serde_json::json!({ "missing_capabilities": missing }),
            )));
        }
    }
    // Refuse activation when existing pages cannot render under the candidate
    // theme editor, so a bad theme cannot break live storefronts.
    if let Some(raw) = target.editor_manifest.as_ref() {
        let editor = ThemeEditorManifest::parse(raw).map_err(|error| {
            ApiError::new(AppError::conflict(format!(
                "stored theme editor manifest is invalid: {error}"
            )))
        })?;
        let issues = ContentService::new(db.clone())
            .validate_all_pages(&editor)
            .await?;
        if !issues.is_empty() {
            return Err(ApiError::new(AppError::conflict_details(
                "theme cannot be activated because store pages do not validate against it",
                serde_json::json!({ "issues": issues }),
            )));
        }
    }
    Ok(())
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
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ThemeDto>> {
    authorize(&actor, "themes.write")?;
    let service = ThemeService::new(db.clone())
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    // Rollback is an activation too: the previous theme must still be safe.
    let previous = service.previous().await?;
    ensure_theme_activatable(&context, &db, &previous).await?;
    let theme = service.rollback().await?;
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
