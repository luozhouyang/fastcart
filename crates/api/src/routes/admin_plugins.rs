//! Admin plugin installation routes.
//!
//! Plugins are validated, stored as content-addressed artifacts, and enabled
//! per store. Mutations are audited; the bundle endpoint used by the runtime
//! lives in [`super::internal_plugin_runtime`].

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use fastcart_http::{AdminActor, ApiError, ApiError as ApiFailure, ApiResult};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{ActorContext, AppError, now_timestamp};
use fastcart_plugin::{
    ArtifactSummary, ContextClaims, InstalledPlugin, PluginDetail, PluginError, PluginManifest,
    PluginSecretName, PluginService, RequestKind, RuntimeRequest, TokenActor,
};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the admin plugin routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/plugins", get(list_plugins))
        .route("/plugins/{id}", get(get_plugin).delete(delete_plugin))
        .route("/plugins/{id}/enable", post(enable_plugin))
        .route("/plugins/{id}/disable", post(disable_plugin))
        .route("/plugins/{id}/settings", patch(update_settings))
        .route("/plugins/{id}/secrets", get(list_secrets))
        .route(
            "/plugins/{id}/secrets/{name}",
            put(set_secret).delete(delete_secret),
        )
        .route("/plugins/{id}/rollback", post(rollback_plugin))
        .route("/plugins/{id}/rpc", post(plugin_rpc))
        .route("/plugins/{id}/deliveries", get(list_deliveries))
        .route(
            "/plugins/{id}/deliveries/{delivery_id}/replay",
            post(replay_delivery),
        )
        .route("/plugins/{id}/jobs", get(list_jobs))
        .route("/plugins/{id}/logs", get(list_logs).delete(clear_logs))
        .route("/plugins/{id}/jobs/{name}/run", post(run_job))
        .route("/plugins/runtime", get(runtime_status))
        .route("/plugins/extensions", get(list_extensions))
}

/// Builds the multipart upload routes merged with their own body limit.
pub fn upload_router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/v1/admin/plugins/install", post(install_plugin))
        .route("/api/v1/admin/plugins/{id}/upgrade", post(upgrade_plugin))
}

/// Documented admin plugin routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_plugins))
        .routes(routes!(get_plugin, delete_plugin))
        .routes(routes!(enable_plugin))
        .routes(routes!(disable_plugin))
        .routes(routes!(update_settings))
        .routes(routes!(rollback_plugin))
}

/// Response for the plugin list.
#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub struct PluginListResponse {
    /// Installed plugins.
    pub plugins: Vec<InstalledPlugin>,
    /// Artifacts present on disk.
    pub artifacts: Vec<ArtifactSummary>,
}

/// Settings update payload.
#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    /// Settings object validated against the manifest schema.
    #[schema(value_type = Object)]
    pub settings: Value,
}

/// Optional upgrade parameters.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct UpgradeQuery {
    /// Accept capabilities added by the new version.
    #[serde(default)]
    pub accept_new_scopes: bool,
}

fn require(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiFailure::new)?;
    actor
        .require_permission(permission)
        .map_err(ApiFailure::new)
}

fn api(error: PluginError) -> ApiError {
    ApiError::new(error.into())
}

fn require_runtime(context: &AppContext) -> ApiResult<()> {
    if context.config.plugins_enabled {
        Ok(())
    } else {
        Err(ApiError::new(AppError::conflict(
            "plugin runtime is disabled",
        )))
    }
}

fn service(context: &AppContext, db: StoreDatabase, actor: &ActorContext) -> PluginService {
    PluginService::new(db, context.plugin_artifacts.clone())
        .with_secret_cipher(context.plugin_secret_cipher.clone())
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
}

/// Plugin lifecycle changes alter the catalog and the resolved grants.
async fn invalidate_after_plugin_change(context: &AppContext, db: &StoreDatabase) {
    super::plugin_catalog::invalidate(context, db.store_id()).await;
}

async fn read_archive(multipart: &mut Multipart, max_bytes: usize) -> ApiResult<Vec<u8>> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::validation(format!("invalid multipart body: {error}")))?
    {
        if field.name() != Some("archive") {
            continue;
        }
        let bytes = field
            .bytes()
            .await
            .map_err(|error| ApiError::validation(format!("failed to read archive: {error}")))?;
        if bytes.is_empty() {
            return Err(ApiError::validation("archive field is empty"));
        }
        if bytes.len() > max_bytes {
            return Err(ApiError::validation(format!(
                "archive exceeds {max_bytes} bytes"
            )));
        }
        return Ok(bytes.to_vec());
    }
    Err(ApiError::validation(
        "multipart field `archive` is required",
    ))
}

#[utoipa::path(
    get,
    path = "/plugins",
    tag = "Plugins",
    summary = "List installed plugins and stored artifacts",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PluginListResponse))
)]
async fn list_plugins(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<PluginListResponse>> {
    require(&actor, "plugins.read")?;
    let service = service(&context, db, &actor);
    let plugins = service.list().await.map_err(api)?;
    let artifacts = service.artifacts().map_err(api)?;
    Ok(Json(PluginListResponse { plugins, artifacts }))
}

#[utoipa::path(
    get,
    path = "/plugins/{id}",
    tag = "Plugins",
    summary = "Get one installed plugin",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses((status = 200, body = PluginDetail))
)]
async fn get_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<PluginDetail>> {
    require(&actor, "plugins.read")?;
    let service = service(&context, db, &actor);
    let (plugin, manifest) = service.detail(&id).await.map_err(api)?;
    Ok(Json(PluginDetail { plugin, manifest }))
}

#[utoipa::path(
    post,
    path = "/plugins/{id}/enable",
    tag = "Plugins",
    summary = "Enable a plugin for this store",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses((status = 200, body = InstalledPlugin))
)]
async fn enable_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InstalledPlugin>> {
    require(&actor, "plugins.write")?;
    require_runtime(&context)?;
    let service = service(&context, db.clone(), &actor);
    let plugin = service.enable(&id).await.map_err(api)?;
    invalidate_after_plugin_change(&context, &db).await;
    Ok(Json(plugin))
}

#[utoipa::path(
    post,
    path = "/plugins/{id}/disable",
    tag = "Plugins",
    summary = "Disable a plugin for this store",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses((status = 200, body = InstalledPlugin))
)]
async fn disable_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InstalledPlugin>> {
    require(&actor, "plugins.write")?;
    let service = service(&context, db.clone(), &actor);
    let plugin = service.disable(&id).await.map_err(api)?;
    invalidate_after_plugin_change(&context, &db).await;
    Ok(Json(plugin))
}

#[utoipa::path(
    post,
    path = "/plugins/{id}/settings",
    tag = "Plugins",
    summary = "Replace plugin settings",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    request_body = UpdateSettingsRequest,
    responses((status = 200, body = InstalledPlugin))
)]
async fn update_settings(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    Json(request): Json<UpdateSettingsRequest>,
) -> ApiResult<Json<InstalledPlugin>> {
    require(&actor, "plugins.write")?;
    let service = service(&context, db.clone(), &actor);
    let plugin = service
        .update_settings(&id, request.settings)
        .await
        .map_err(api)?;
    // The cached catalog carries settings for the admin extension payload.
    invalidate_after_plugin_change(&context, &db).await;
    Ok(Json(plugin))
}

/// Secret replacement payload.
#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub struct SetSecretRequest {
    /// Plaintext secret value. It is never returned by the API.
    pub value: String,
}

/// Lists stored secret names for one plugin.
///
/// Values are write-only and never included.
async fn list_secrets(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<PluginSecretName>>> {
    require(&actor, "plugins.write")?;
    let service = service(&context, db, &actor);
    Ok(Json(service.list_secret_names(&id).await.map_err(api)?))
}

/// Creates or replaces one plugin secret.
async fn set_secret(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, name)): Path<(String, String)>,
    Json(request): Json<SetSecretRequest>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    if !context.plugin_secrets_durable {
        return Err(ApiError::new(AppError::conflict(
            "plugin secrets require [plugins] context_secret in the server configuration",
        )));
    }
    let service = service(&context, db, &actor);
    service
        .set_secret(&id, &name, request.value.as_bytes())
        .await
        .map_err(api)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes one plugin secret.
async fn delete_secret(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    let service = service(&context, db, &actor);
    service.delete_secret(&id, &name).await.map_err(api)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/plugins/{id}/rollback",
    tag = "Plugins",
    summary = "Roll back to the previous artifact",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses((status = 200, body = InstalledPlugin))
)]
async fn rollback_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InstalledPlugin>> {
    require(&actor, "plugins.write")?;
    let service = service(&context, db.clone(), &actor);
    let plugin = service.rollback(&id).await.map_err(api)?;
    invalidate_after_plugin_change(&context, &db).await;
    Ok(Json(plugin))
}

#[utoipa::path(
    delete,
    path = "/plugins/{id}",
    tag = "Plugins",
    summary = "Remove a disabled plugin",
    security(("adminSession" = []), ("apiToken" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses((status = 204))
)]
async fn delete_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.install")?;
    let service = service(&context, db.clone(), &actor);
    service.remove(&id).await.map_err(api)?;
    invalidate_after_plugin_change(&context, &db).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn install_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<InstalledPlugin>)> {
    require(&actor, "plugins.install")?;
    require_runtime(&context)?;
    let bytes = read_archive(&mut multipart, context.config.max_plugin_archive_bytes).await?;
    let service = service(&context, db, &actor);
    match service.install(&bytes).await {
        Ok(plugin) => Ok((StatusCode::CREATED, Json(plugin))),
        Err(error) => {
            service.record_failure("plugin.install", &error).await;
            Err(api(error))
        }
    }
}

async fn upgrade_plugin(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    Query(query): Query<UpgradeQuery>,
    mut multipart: Multipart,
) -> ApiResult<Json<InstalledPlugin>> {
    require(&actor, "plugins.install")?;
    require_runtime(&context)?;
    let bytes = read_archive(&mut multipart, context.config.max_plugin_archive_bytes).await?;
    let service = service(&context, db.clone(), &actor);
    match service.upgrade(&id, &bytes, query.accept_new_scopes).await {
        Ok(plugin) => {
            invalidate_after_plugin_change(&context, &db).await;
            Ok(Json(plugin))
        }
        Err(error) => {
            service.record_failure("plugin.upgrade", &error).await;
            Err(api(error))
        }
    }
}

/// JSON-RPC envelope accepted from the admin SPA.
#[derive(Clone, Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct RpcRequest {
    /// Declared plugin method.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: Value,
}

async fn plugin_rpc(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    Json(request): Json<RpcRequest>,
) -> ApiResult<axum::response::Response> {
    require(&actor, "plugins.read")?;
    require_runtime(&context)?;
    if !valid_method(&request.method) {
        return Err(ApiError::validation("invalid plugin method"));
    }
    let row: Option<(String, i64, String, String)> = sqlx::query_as(
        "SELECT digest, enabled, granted_scopes, manifest_json FROM plugin_installations          WHERE plugin_id = ?",
    )
    .bind(&id)
    .fetch_optional(db.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some((digest, enabled, scopes_json, manifest_json)) = row else {
        return Err(ApiError::new(AppError::not_found("plugin", id)));
    };
    if enabled == 0 {
        return Err(ApiError::new(AppError::Conflict(format!(
            "plugin is disabled: {id}"
        ))));
    }
    let manifest = fastcart_plugin::PluginManifest::parse(manifest_json.as_bytes())
        .map_err(|error| ApiError::new(error.into()))?;
    if !manifest.rpc.iter().any(|method| method == &request.method) {
        return Err(ApiError::new(AppError::not_found(
            "plugin method",
            request.method,
        )));
    }
    let required = rpc_permission(&manifest, &request.method).unwrap_or("plugins.write");
    if !actor.has_permission(required) {
        return Err(ApiError::new(AppError::Forbidden));
    }
    let Some(host) = context.plugin_host.as_ref() else {
        return Ok((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "plugin runtime unavailable",
                "code": "plugin_unavailable",
            })),
        )
            .into_response());
    };
    let scopes: Vec<String> = serde_json::from_str::<Vec<String>>(&scopes_json)
        .unwrap_or_default()
        .into_iter()
        .filter(|scope| admin_scope_allowed(scope, &actor))
        .collect();
    let user_id = actor
        .user_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let digest_for_token = digest.clone();
    let claims = ContextClaims::new(
        id.clone(),
        db.store_id().as_str(),
        digest_for_token.clone(),
        scopes,
        TokenActor::Admin {
            id: user_id,
            permissions: actor.permissions.iter().cloned().collect(),
        },
        now_timestamp(),
    );
    let token = context
        .plugin_tokens
        .derive(db.store_id().as_str(), &id, &digest_for_token)
        .mint(&claims)
        .map_err(|error| ApiError::new(AppError::internal(error.to_string())))?;
    let response = host
        .forward(RuntimeRequest {
            plugin_id: claims.sub.clone(),
            digest,
            store_id: claims.store.clone(),
            path_and_query: format!("/admin/{}", request.method),
            method: "POST".to_owned(),
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            context_token: token,
            body: bytes::Bytes::from(request.params.to_string()),
            kind: RequestKind::Admin,
            deadline_override: None,
        })
        .await;
    super::plugin_runtime::record_forward(&context, &response);
    super::plugin_runtime::forward_outcome(response)
}

/// Returns the permission required to invoke one declared RPC method.
///
/// A manifest may declare the permission explicitly. Otherwise the method
/// prefix supplies a conservative default, and an undeclared method falls back
/// to `plugins.write` so `plugins.read` alone cannot trigger plugin code.
fn rpc_permission<'a>(manifest: &'a PluginManifest, method: &str) -> Option<&'a str> {
    if let Some(permission) = manifest.rpc_permissions.get(method) {
        return Some(permission.as_str());
    }
    match method.split('.').next().unwrap_or_default() {
        "orders" => Some("orders.read"),
        "products" => Some("products.read"),
        "customers" => Some("customers.read"),
        "settings" => Some("settings.read"),
        "plugins" => Some("plugins.read"),
        _ => None,
    }
}

/// Returns whether the acting admin may exercise one plugin scope.
///
/// The context token carries the intersection of the plugin's granted scopes
/// and the user's permissions, so a plugin method cannot read orders or
/// customers on behalf of an admin who lacks those permissions.
fn admin_scope_allowed(scope: &str, actor: &ActorContext) -> bool {
    let has = |permissions: &[&str]| {
        permissions
            .iter()
            .any(|permission| actor.has_permission(permission))
    };
    match scope {
        "catalog.read" => has(&["products.read", "products.write"]),
        "orders.read" | "carts.read" => has(&["orders.read", "orders.write"]),
        "customers.read" => has(&["customers.read", "customers.write"]),
        "content.read" => has(&["themes.read", "themes.write"]),
        "secrets.read" => has(&["plugins.write", "plugins.install"]),
        _ => true,
    }
}

fn valid_method(method: &str) -> bool {
    !method.is_empty()
        && method.len() <= 64
        && method
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

async fn runtime_status(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
) -> ApiResult<Json<super::plugin_runtime::RuntimeStatus>> {
    require(&actor, "plugins.read")?;
    Ok(Json(super::plugin_runtime::status(&context).await))
}

/// One admin extension entry exposed to the SPA.
#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub struct AdminExtensionView {
    /// Target slot.
    pub target: String,
    /// Merchant-facing title.
    pub title: Option<String>,
    /// Navigation label.
    pub label: Option<String>,
    /// Icon name.
    pub icon: Option<String>,
    /// Navigation route.
    pub route: Option<String>,
    /// HTML entry path relative to the artifact.
    pub entry: Option<String>,
    /// Signed iframe URL for HTML entries.
    pub src: Option<String>,
    /// Preferred height.
    pub height: Option<u32>,
    /// Sort order.
    pub priority: Option<i32>,
    /// Permission required to render the slot.
    pub permission: String,
    /// Owning plugin id.
    pub plugin_id: String,
    /// Owning plugin name.
    pub plugin_name: String,
    /// Owning plugin version.
    pub plugin_version: String,
    /// Artifact digest.
    pub digest: String,
    /// Published plugin settings.
    #[schema(value_type = Object)]
    pub settings: Value,
}

/// Response for the admin extension manifest.
#[derive(Clone, Debug, serde::Serialize, ToSchema)]
pub struct AdminExtensionsResponse {
    /// Flat extension list across enabled plugins.
    pub extensions: Vec<AdminExtensionView>,
}

async fn list_extensions(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<AdminExtensionsResponse>> {
    require(&actor, "plugins.read")?;
    let actor_id = actor
        .user_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let catalog = super::plugin_catalog::load(&context, &db)
        .await
        .map_err(ApiError::new)?;
    let mut extensions = Vec::new();
    for plugin in &catalog.plugins {
        let plugin_id = &plugin.plugin_id;
        let digest = &plugin.digest;
        let manifest = &plugin.manifest;
        let settings: Value = serde_json::from_str(&plugin.settings_json).unwrap_or(Value::Null);
        let token = super::plugin_ui::mount_token(
            &context,
            db.store_id().as_str(),
            plugin_id,
            digest,
            &actor_id,
        );
        for extension in &manifest.extensions.admin {
            let permission = fastcart_plugin::targets::required_permission(&extension.target)
                .unwrap_or("plugins.read");
            let src = extension.entry.as_ref().and_then(|entry| {
                token.as_ref().map(|token| {
                    format!(
                        "/_fastcart/plugin-ui/{}/{}/{}/{}?t={}",
                        db.store_id().as_str(),
                        plugin_id,
                        digest,
                        entry.trim_start_matches('/'),
                        token
                    )
                })
            });
            extensions.push(AdminExtensionView {
                target: extension.target.clone(),
                title: extension.title.clone(),
                label: extension.label.clone(),
                icon: extension.icon.clone(),
                route: extension.route.clone(),
                entry: extension.entry.clone(),
                src,
                height: extension.height,
                priority: extension.priority,
                permission: permission.to_owned(),
                plugin_id: plugin_id.clone(),
                plugin_name: manifest.name.clone(),
                plugin_version: manifest.version.clone(),
                digest: digest.clone(),
                settings: settings.clone(),
            });
        }
    }
    Ok(Json(AdminExtensionsResponse { extensions }))
}

/// One persisted plugin delivery.
#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow, ToSchema)]
pub struct DeliveryView {
    pub id: String,
    pub plugin_id: String,
    pub event_id: String,
    pub event_type: String,
    pub status: String,
    pub attempts: i64,
    pub available_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// One persisted plugin job.
#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow, ToSchema)]
pub struct JobView {
    pub id: String,
    pub plugin_id: String,
    pub name: String,
    pub status: String,
    pub attempts: i64,
    pub available_at: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

async fn list_deliveries(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<DeliveryView>>> {
    require(&actor, "plugins.read")?;
    let rows: Vec<DeliveryView> = sqlx::query_as(
        "SELECT id, plugin_id, event_id, event_type, status, attempts, available_at,          last_error, created_at FROM plugin_deliveries WHERE plugin_id = ?          ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&id)
    .fetch_all(db.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(Json(rows))
}

async fn replay_delivery(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, delivery_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    fastcart_plugin::PluginDispatch::replay_delivery(&db, &id, &delivery_id)
        .await
        .map_err(api)?;
    Ok(StatusCode::ACCEPTED)
}

async fn list_jobs(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<JobView>>> {
    require(&actor, "plugins.read")?;
    let rows: Vec<JobView> = sqlx::query_as(
        "SELECT id, plugin_id, name, status, attempts, available_at, last_error,          created_at, updated_at FROM plugin_jobs WHERE plugin_id = ?          ORDER BY created_at DESC LIMIT 50",
    )
    .bind(&id)
    .fetch_all(db.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(Json(rows))
}

async fn run_job(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, name)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    let manifest_json: Option<String> = sqlx::query_scalar(
        "SELECT manifest_json FROM plugin_installations WHERE plugin_id = ? AND enabled = 1",
    )
    .bind(&id)
    .fetch_optional(db.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    let Some(manifest_json) = manifest_json else {
        return Err(ApiError::new(AppError::not_found("plugin", id)));
    };
    let manifest = PluginManifest::parse(manifest_json.as_bytes())
        .map_err(|error| ApiError::new(error.into()))?;
    if !manifest.jobs.iter().any(|job| job.name == name) {
        return Err(ApiError::new(AppError::not_found("plugin job", name)));
    }
    fastcart_plugin::PluginDispatch::enqueue_job(&db, &id, &name)
        .await
        .map_err(api)?;
    Ok(StatusCode::ACCEPTED)
}

/// One persisted plugin log line.
#[derive(Clone, Debug, serde::Serialize, sqlx::FromRow, ToSchema)]
pub struct LogView {
    pub id: i64,
    pub plugin_id: String,
    pub level: String,
    pub message: String,
    pub fields_json: Option<String>,
    pub created_at: i64,
}

async fn list_logs(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<LogView>>> {
    require(&actor, "plugins.read")?;
    let rows: Vec<LogView> = sqlx::query_as(
        "SELECT id, plugin_id, level, message, fields_json, created_at FROM plugin_logs          WHERE plugin_id = ? ORDER BY id DESC LIMIT 200",
    )
    .bind(&id)
    .fetch_all(db.pool())
    .await
    .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(Json(rows))
}

async fn clear_logs(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require(&actor, "plugins.write")?;
    sqlx::query("DELETE FROM plugin_logs WHERE plugin_id = ?")
        .bind(&id)
        .execute(db.pool())
        .await
        .map_err(|error| ApiError::new(AppError::database(error.to_string())))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastcart_kernel::ActorContext;
    use serde_json::json;

    fn manifest() -> PluginManifest {
        PluginManifest::parse(
            json!({
                "schema_version": "1",
                "id": "demo",
                "name": "Demo",
                "version": "1.0.0",
                "api_version": 2,
                "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
                "entry": { "type": "worker", "path": "worker/index.js" },
                "capabilities": {
                    "data": ["catalog.read", "orders.read", "customers.read", "content.read"],
                    "storage": ["kv.read", "kv.write"],
                    "secrets": ["secrets.read"]
                },
                "rpc": ["conversations.list", "orders.refund"],
                "rpc_permissions": { "conversations.list": "plugins.write" }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap()
    }

    fn actor(permissions: &[&str]) -> ActorContext {
        ActorContext {
            permissions: permissions
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            ..ActorContext::default()
        }
    }

    #[test]
    fn rpc_permissions_prefer_declarations_then_prefixes() {
        let manifest = manifest();
        assert_eq!(
            rpc_permission(&manifest, "conversations.list"),
            Some("plugins.write")
        );
        assert_eq!(
            rpc_permission(&manifest, "orders.refund"),
            Some("orders.read")
        );
        assert_eq!(rpc_permission(&manifest, "unknown.method"), None);
    }

    #[test]
    fn admin_scopes_follow_the_acting_users_permissions() {
        let limited = actor(&["plugins.read"]);
        assert!(!admin_scope_allowed("catalog.read", &limited));
        assert!(!admin_scope_allowed("orders.read", &limited));
        assert!(!admin_scope_allowed("customers.read", &limited));
        assert!(!admin_scope_allowed("content.read", &limited));
        assert!(!admin_scope_allowed("secrets.read", &limited));
        assert!(admin_scope_allowed("kv.read", &limited));

        let full = actor(&[
            "products.read",
            "orders.read",
            "customers.read",
            "themes.read",
            "plugins.write",
        ]);
        for scope in [
            "catalog.read",
            "orders.read",
            "carts.read",
            "customers.read",
            "content.read",
            "secrets.read",
            "kv.read",
        ] {
            assert!(admin_scope_allowed(scope, &full), "{scope}");
        }
    }
}
