//! Admin content HTTP routes: pages, versions, navigation and redirects.
//!
//! Handlers enforce `themes.*` permissions and delegate all business rules to
//! [`ContentService`].

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{ActorContext, AppError, Metadata, NavigationId, PageId, now_timestamp};
use fastcart_module_content::{
    ContentService, CreateNavigationInput, CreatePageInput, CreateRedirectInput, Navigation,
    NavigationItem, NavigationItemInput, NavigationTree, Page, PageRevisionOrigin, PageStatus,
    PageVersion, Redirect, UpdateNavigationInput, UpdatePageInput,
};
use fastcart_module_theme::ThemeService;
use fastcart_storefront::{PageThemeRef, SUPPORTED_EDITOR_SCHEMA_VERSION, ThemeEditorManifest};
use serde::Deserialize;
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{StoreDb, record_store_audit};
use crate::dto::PageResponse;
use crate::routes::preview::{issue_preview_token, valid_secret};
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/content/pages", get(list_pages).post(create_page))
        .route(
            "/content/pages/{id}",
            get(get_page).patch(update_page).delete(delete_page),
        )
        .route("/content/pages/{id}/publish", post(publish_page))
        .route("/content/pages/{id}/unpublish", post(unpublish_page))
        .route("/content/pages/{id}/archive", post(archive_page))
        .route(
            "/content/pages/{id}/preview-token",
            post(create_preview_token),
        )
        .route(
            "/content/pages/{id}/preview-token/revoke",
            post(revoke_preview_token),
        )
        .route("/content/pages/{id}/versions", get(list_page_versions))
        .route(
            "/content/pages/{id}/versions/{version}/restore",
            post(restore_page_version),
        )
        .route(
            "/content/navigations",
            get(list_navigations).post(create_navigation),
        )
        .route(
            "/content/navigations/{id}",
            get(get_navigation)
                .patch(update_navigation)
                .delete(delete_navigation),
        )
        .route("/content/navigations/{id}/tree", get(get_navigation_tree))
        .route("/content/navigations/{id}/items", post(add_navigation_item))
        .route(
            "/content/navigation-items/{item_id}",
            patch(update_navigation_item).delete(delete_navigation_item),
        )
        .route(
            "/content/navigations/{id}/items/reorder",
            post(reorder_navigation_items),
        )
        .route(
            "/content/redirects",
            get(list_redirects).post(create_redirect),
        )
        .route(
            "/content/redirects/{id}",
            get(get_redirect).delete(delete_redirect),
        )
}

/// Builds the documented admin content routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_pages, create_page))
        .routes(routes!(get_page, update_page, delete_page))
        .routes(routes!(publish_page))
        .routes(routes!(unpublish_page))
        .routes(routes!(archive_page))
        .routes(routes!(create_preview_token))
        .routes(routes!(revoke_preview_token))
        .routes(routes!(list_page_versions))
        .routes(routes!(restore_page_version))
        .routes(routes!(list_navigations, create_navigation))
        .routes(routes!(
            get_navigation,
            update_navigation,
            delete_navigation
        ))
        .routes(routes!(get_navigation_tree))
        .routes(routes!(add_navigation_item))
        .routes(routes!(update_navigation_item, delete_navigation_item))
        .routes(routes!(reorder_navigation_items))
        .routes(routes!(list_redirects, create_redirect))
        .routes(routes!(get_redirect, delete_redirect))
}

/// Query parameters for administrative page listing.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct PageListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    include_deleted: bool,
}

/// Navigation item reorder body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct NavigationItemReorderRequest {
    item_ids: Vec<String>,
}

/// Page creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreatePageRequest {
    title: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    page_type: Option<String>,
    #[serde(default)]
    body: Value,
    #[serde(default)]
    seo: Value,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreatePageRequest> for CreatePageInput {
    fn from(request: CreatePageRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            page_type: request.page_type,
            body: request.body,
            seo: request.seo,
            metadata: request.metadata,
        }
    }
}

/// Partial page update body.
///
/// HTTP updates are optimistic: callers must send the revision they observed.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdatePageRequest {
    /// Revision the caller last observed.
    base_revision: i64,
    /// Revision author, either `manual` or `ai`.
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    page_type: Option<String>,
    #[serde(default)]
    body: Option<Value>,
    #[serde(default)]
    seo: Option<Value>,
    #[serde(default)]
    metadata: Option<Metadata>,
}

impl TryFrom<UpdatePageRequest> for UpdatePageInput {
    type Error = ApiError;

    fn try_from(request: UpdatePageRequest) -> Result<Self, Self::Error> {
        let origin = request
            .origin
            .as_deref()
            .map(parse_page_revision_origin)
            .transpose()?;
        Ok(Self {
            title: request.title,
            handle: request.handle,
            page_type: request.page_type,
            body: request.body,
            seo: request.seo,
            metadata: request.metadata,
            base_revision: Some(request.base_revision),
            origin,
            created_by: None,
        })
    }
}

/// Navigation creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateNavigationRequest {
    name: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateNavigationRequest> for CreateNavigationInput {
    fn from(request: CreateNavigationRequest) -> Self {
        Self {
            name: request.name,
            handle: request.handle,
            metadata: request.metadata,
        }
    }
}

/// Partial navigation update body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateNavigationRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    metadata: Option<Metadata>,
}

impl From<UpdateNavigationRequest> for UpdateNavigationInput {
    fn from(request: UpdateNavigationRequest) -> Self {
        Self {
            name: request.name,
            handle: request.handle,
            metadata: request.metadata,
        }
    }
}

/// Navigation item create/update body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct NavigationItemRequest {
    #[serde(default)]
    parent_id: Option<String>,
    title: String,
    url: String,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<NavigationItemRequest> for NavigationItemInput {
    fn from(request: NavigationItemRequest) -> Self {
        Self {
            parent_id: request.parent_id,
            title: request.title,
            url: request.url,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

/// Redirect creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRedirectRequest {
    from_path: String,
    to_path: String,
    #[serde(default)]
    status_code: Option<i64>,
}

impl From<CreateRedirectRequest> for CreateRedirectInput {
    fn from(request: CreateRedirectRequest) -> Self {
        Self {
            from_path: request.from_path,
            to_path: request.to_path,
            status_code: request.status_code,
        }
    }
}

/// Parses a page status filter strictly.
fn parse_page_status(value: &str) -> ApiResult<PageStatus> {
    PageStatus::parse(value).ok_or_else(|| ApiError::validation("invalid page status"))
}

/// Parses a page revision origin strictly.
fn parse_page_revision_origin(value: &str) -> ApiResult<PageRevisionOrigin> {
    PageRevisionOrigin::parse(value)
        .ok_or_else(|| ApiError::validation("invalid page revision origin"))
}

/// Builds a content service bound to the active theme.
///
/// Page bodies are validated against the active theme editor manifest and
/// stamped with a [`PageThemeRef`] built from the active manifest identity. The
/// actor is recorded by the service inside each mutation transaction, so
/// content audits are atomic with the change and never need a second write.
async fn content_service(db: StoreDatabase, actor_id: Option<String>) -> ApiResult<ContentService> {
    let (editor_manifest, theme_ref) = match ThemeService::new(db.clone()).active().await? {
        Some(theme) => {
            let editor_manifest = match theme.editor_manifest {
                Some(raw) => Some(ThemeEditorManifest::parse(&raw).map_err(|error| {
                    AppError::conflict(format!("stored theme editor manifest is invalid: {error}"))
                })?),
                None => None,
            };
            let theme_ref = PageThemeRef {
                id: theme.manifest.id,
                version: theme.manifest.version,
                schema_version: SUPPORTED_EDITOR_SCHEMA_VERSION,
            };
            (editor_manifest, Some(theme_ref))
        }
        None => (None, None),
    };
    Ok(ContentService::with_editor_manifest(db, editor_manifest)
        .with_theme_ref(theme_ref)
        .with_audit_actor(actor_id))
}

/// Returns the authenticated actor identifier, when one is available.
fn actor_id(actor: &ActorContext) -> Option<String> {
    actor.user_id.as_ref().map(|id| id.as_str().to_owned())
}

/// Enforces a permission at the handler boundary.
fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

// -------------------------------------------------------------------------
// Pages
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/content/pages",
    tag = "Admin",
    summary = "List content pages",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, PageListQuery),
    responses((status = 200, body = PageResponse<Page>))
)]
async fn list_pages(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<PageListQuery>,
) -> ApiResult<Json<PageResponse<Page>>> {
    authorize(&actor, "themes.read")?;
    let status = query.status.as_deref().map(parse_page_status).transpose()?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ContentService::new(db)
        .list_pages(status, query.include_deleted, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/content/pages",
    tag = "Admin",
    summary = "Create a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreatePageRequest,
    responses((status = 201, body = Page))
)]
async fn create_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<CreatePageRequest>,
) -> ApiResult<(StatusCode, Json<Page>)> {
    authorize(&actor, "themes.write")?;
    let page = content_service(db, actor_id(&actor))
        .await?
        .create_page(input.into())
        .await?;
    Ok((StatusCode::CREATED, Json(page)))
}

#[utoipa::path(
    get,
    path = "/content/pages/{id}",
    tag = "Admin",
    summary = "Get a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Page))
)]
async fn get_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.read")?;
    let page_id = PageId::from_string(id);
    let page = ContentService::new(db).get_page(&page_id).await?;
    Ok(Json(page))
}

#[utoipa::path(
    patch,
    path = "/content/pages/{id}",
    tag = "Admin",
    summary = "Update a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdatePageRequest,
    responses((status = 200, body = Page))
)]
async fn update_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<UpdatePageRequest>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let mut input = UpdatePageInput::try_from(input)?;
    input.created_by = actor_id(&actor);
    let page = content_service(db, actor_id(&actor))
        .await?
        .update_page(&page_id, input)
        .await?;
    Ok(Json(page))
}

#[utoipa::path(
    delete,
    path = "/content/pages/{id}",
    tag = "Admin",
    summary = "Delete a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    content_service(db, actor_id(&actor))
        .await?
        .delete_page(&page_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/publish",
    tag = "Admin",
    summary = "Publish a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Page))
)]
async fn publish_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let created_by = actor_id(&actor);
    let page = content_service(db, created_by.clone())
        .await?
        .publish_page_as(&page_id, created_by.as_deref(), PageRevisionOrigin::Manual)
        .await?;
    Ok(Json(page))
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/unpublish",
    tag = "Admin",
    summary = "Unpublish a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Page))
)]
async fn unpublish_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let page = content_service(db, actor_id(&actor))
        .await?
        .unpublish_page(&page_id)
        .await?;
    Ok(Json(page))
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/archive",
    tag = "Admin",
    summary = "Archive a content page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Page))
)]
async fn archive_page(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let page = content_service(db, actor_id(&actor))
        .await?
        .archive_page(&page_id)
        .await?;
    Ok(Json(page))
}

/// A short-lived capability for rendering one page in draft preview.
///
/// The token is returned only to the authenticated administrator and must not
/// be included in audit logs.
#[derive(Clone, Debug, serde::Serialize, utoipa::ToSchema)]
pub struct PreviewTokenResponse {
    /// Signed `payload.signature` preview token.
    pub token: String,
    /// Unix timestamp when the token expires.
    pub expires_at: i64,
    /// Identifier that can be revoked before the token expires.
    pub jti: String,
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/preview-token",
    tag = "Admin",
    summary = "Issue a draft preview token for a page",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PreviewTokenResponse))
)]
async fn create_preview_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<(
    [(axum::http::HeaderName, axum::http::HeaderValue); 1],
    Json<PreviewTokenResponse>,
)> {
    authorize(&actor, "themes.read")?;
    let Some(secret) = context
        .config
        .preview_secret
        .as_deref()
        .filter(|value| valid_secret(value))
    else {
        return Err(ApiError::new(AppError::NotImplemented(
            "page preview is not configured".to_owned(),
        )));
    };
    let page_id = PageId::from_string(id);
    // Drafts are allowed; archived and deleted pages are rejected because the
    // preview surface refuses to render them.
    let page = ContentService::new(db.clone()).get_page(&page_id).await?;
    if page.status == PageStatus::Archived {
        return Err(ApiError::not_found("page"));
    }
    let issued = issue_preview_token(
        secret,
        db.store_id().as_str(),
        page_id.as_str(),
        now_timestamp(),
    )?;
    // Never record the token itself: it is a bearer credential valid for the
    // whole TTL, so the audit entry only records that one was issued.
    record_store_audit(
        &context,
        &actor,
        "page.preview_token.issued",
        "page",
        page_id.as_str(),
        serde_json::json!({ "expires_at": issued.expires_at, "jti": issued.jti }),
    )
    .await;
    // A response containing a bearer token must not be retained by browsers,
    // proxies or shared caches.
    let headers = [(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    )];
    Ok((
        headers,
        Json(PreviewTokenResponse {
            token: issued.token,
            expires_at: issued.expires_at,
            jti: issued.jti,
        }),
    ))
}

/// Body of a preview-token revocation request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokePreviewTokenRequest {
    /// Identifier returned when the token was issued.
    pub jti: String,
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/preview-token/revoke",
    tag = "Admin",
    summary = "Revoke a draft preview token",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = RevokePreviewTokenRequest,
    responses((status = 204))
)]
async fn revoke_preview_token(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<RevokePreviewTokenRequest>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let jti = input.jti.trim();
    if jti.is_empty() || jti.len() > 128 {
        return Err(ApiError::validation(
            "jti must be between 1 and 128 characters",
        ));
    }
    // Only let an administrator of this store revoke against a page they can
    // load; the jti itself is opaque and carries no page authority.
    ContentService::new(db.clone()).get_page(&page_id).await?;
    let now = now_timestamp();
    let store_id = db.store_id().as_str().to_owned();
    let asset_authorization_key = crate::routes::preview::asset_authorization_key(&store_id, jti);
    let mut tx = db.begin().await.map_err(AppError::from)?;
    sqlx::query("DELETE FROM preview_token_revocations WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(fastcart_infrastructure::database_error)?;
    sqlx::query(
        "INSERT OR REPLACE INTO preview_token_revocations (jti, page_id, expires_at) \
         VALUES (?, ?, ?)",
    )
    .bind(jti)
    .bind(page_id.as_str())
    .bind(now.saturating_add(crate::routes::preview::PREVIEW_TOKEN_TTL_SECONDS))
    .execute(&mut *tx)
    .await
    .map_err(fastcart_infrastructure::database_error)?;
    if let Some(user_id) = actor.user_id.as_ref() {
        fastcart_infrastructure::AuditActor::new(user_id.to_string(), store_id)
            .append(
                &mut *tx,
                "page.preview_token.revoked",
                "page",
                page_id.as_str(),
                serde_json::json!({ "jti": jti }),
            )
            .await
            .map_err(fastcart_infrastructure::database_error)?;
    }
    tx.commit()
        .await
        .map_err(fastcart_infrastructure::database_error)?;
    context
        .preview_asset_authorizations
        .invalidate(&asset_authorization_key)
        .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/content/pages/{id}/versions",
    tag = "Admin",
    summary = "List page versions",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<PageVersion>))
)]
async fn list_page_versions(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<PageVersion>>> {
    authorize(&actor, "themes.read")?;
    let page_id = PageId::from_string(id);
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let versions = ContentService::new(db)
        .list_page_versions(&page_id, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(versions)))
}

/// Optional optimistic revision for a page restore.
#[derive(Clone, Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct RestorePageVersionQuery {
    /// Revision the caller last observed.
    #[serde(default)]
    base_revision: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/content/pages/{id}/versions/{version}/restore",
    tag = "Admin",
    summary = "Restore a page version",
    security(("adminSession" = []), ("apiToken" = [])),
    params(RestorePageVersionQuery),
    responses((status = 200, body = Page))
)]
async fn restore_page_version(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, version)): Path<(String, i64)>,
    ApiQuery(query): ApiQuery<RestorePageVersionQuery>,
) -> ApiResult<Json<Page>> {
    authorize(&actor, "themes.write")?;
    let page_id = PageId::from_string(id);
    let created_by = actor_id(&actor);
    let page = content_service(db, created_by.clone())
        .await?
        .restore_version_as(
            &page_id,
            version,
            query.base_revision,
            created_by.as_deref(),
            PageRevisionOrigin::Manual,
        )
        .await?;
    Ok(Json(page))
}

// -------------------------------------------------------------------------
// Navigation
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/content/navigations",
    tag = "Admin",
    summary = "List navigations",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Navigation>))
)]
async fn list_navigations(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Navigation>>> {
    authorize(&actor, "themes.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ContentService::new(db).list_navigations(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/content/navigations",
    tag = "Admin",
    summary = "Create a navigation",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateNavigationRequest,
    responses((status = 201, body = Navigation))
)]
async fn create_navigation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<CreateNavigationRequest>,
) -> ApiResult<(StatusCode, Json<Navigation>)> {
    authorize(&actor, "themes.write")?;
    let navigation = content_service(db, actor_id(&actor))
        .await?
        .create_navigation(input.into())
        .await?;
    Ok((StatusCode::CREATED, Json(navigation)))
}

#[utoipa::path(
    get,
    path = "/content/navigations/{id}",
    tag = "Admin",
    summary = "Get a navigation",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Navigation))
)]
async fn get_navigation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Navigation>> {
    authorize(&actor, "themes.read")?;
    let navigation_id = NavigationId::from_string(id);
    let navigation = ContentService::new(db)
        .get_navigation(&navigation_id)
        .await?;
    Ok(Json(navigation))
}

#[utoipa::path(
    patch,
    path = "/content/navigations/{id}",
    tag = "Admin",
    summary = "Update a navigation",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateNavigationRequest,
    responses((status = 200, body = Navigation))
)]
async fn update_navigation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<UpdateNavigationRequest>,
) -> ApiResult<Json<Navigation>> {
    authorize(&actor, "themes.write")?;
    let navigation_id = NavigationId::from_string(id);
    let navigation = content_service(db, actor_id(&actor))
        .await?
        .update_navigation(&navigation_id, input.into())
        .await?;
    Ok(Json(navigation))
}

#[utoipa::path(
    delete,
    path = "/content/navigations/{id}",
    tag = "Admin",
    summary = "Delete a navigation",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_navigation(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let navigation_id = NavigationId::from_string(id);
    content_service(db, actor_id(&actor))
        .await?
        .delete_navigation(&navigation_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/content/navigations/{id}/tree",
    tag = "Admin",
    summary = "Get a navigation tree",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = NavigationTree))
)]
async fn get_navigation_tree(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<NavigationTree>> {
    authorize(&actor, "themes.read")?;
    let navigation_id = NavigationId::from_string(id);
    let tree = ContentService::new(db)
        .get_navigation_tree_by_id(&navigation_id)
        .await?;
    Ok(Json(tree))
}

#[utoipa::path(
    post,
    path = "/content/navigations/{id}/items",
    tag = "Admin",
    summary = "Add a navigation item",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = NavigationItemRequest,
    responses((status = 201, body = NavigationItem))
)]
async fn add_navigation_item(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<NavigationItemRequest>,
) -> ApiResult<(StatusCode, Json<NavigationItem>)> {
    authorize(&actor, "themes.write")?;
    let navigation_id = NavigationId::from_string(id);
    let item = content_service(db, actor_id(&actor))
        .await?
        .add_navigation_item(&navigation_id, input.into())
        .await?;
    Ok((StatusCode::CREATED, Json(item)))
}

#[utoipa::path(
    patch,
    path = "/content/navigation-items/{item_id}",
    tag = "Admin",
    summary = "Update a navigation item",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = NavigationItemRequest,
    responses((status = 200, body = NavigationItem))
)]
async fn update_navigation_item(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(item_id): Path<String>,
    ApiJson(input): ApiJson<NavigationItemRequest>,
) -> ApiResult<Json<NavigationItem>> {
    authorize(&actor, "themes.write")?;
    let item = content_service(db, actor_id(&actor))
        .await?
        .update_navigation_item(&item_id, input.into())
        .await?;
    Ok(Json(item))
}

#[utoipa::path(
    delete,
    path = "/content/navigation-items/{item_id}",
    tag = "Admin",
    summary = "Delete a navigation item",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_navigation_item(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(item_id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    content_service(db, actor_id(&actor))
        .await?
        .remove_navigation_item(&item_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/content/navigations/{id}/items/reorder",
    tag = "Admin",
    summary = "Reorder navigation items",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = NavigationItemReorderRequest,
    responses((status = 200, body = Vec<NavigationItem>))
)]
async fn reorder_navigation_items(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<NavigationItemReorderRequest>,
) -> ApiResult<Json<Vec<NavigationItem>>> {
    authorize(&actor, "themes.write")?;
    let navigation_id = NavigationId::from_string(id);
    let items = content_service(db, actor_id(&actor))
        .await?
        .reorder_navigation_items(&navigation_id, &request.item_ids)
        .await?;
    Ok(Json(items))
}

// -------------------------------------------------------------------------
// Redirects
// -------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/content/redirects",
    tag = "Admin",
    summary = "List redirects",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Redirect>))
)]
async fn list_redirects(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Redirect>>> {
    authorize(&actor, "themes.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ContentService::new(db).list_redirects(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/content/redirects",
    tag = "Admin",
    summary = "Create a redirect",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateRedirectRequest,
    responses((status = 201, body = Redirect))
)]
async fn create_redirect(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(input): ApiJson<CreateRedirectRequest>,
) -> ApiResult<(StatusCode, Json<Redirect>)> {
    authorize(&actor, "themes.write")?;
    let redirect = content_service(db, actor_id(&actor))
        .await?
        .create_redirect(input.into())
        .await?;
    Ok((StatusCode::CREATED, Json(redirect)))
}

#[utoipa::path(
    get,
    path = "/content/redirects/{id}",
    tag = "Admin",
    summary = "Get a redirect",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Redirect))
)]
async fn get_redirect(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Redirect>> {
    authorize(&actor, "themes.read")?;
    let redirect = ContentService::new(db).get_redirect(&id).await?;
    Ok(Json(redirect))
}

#[utoipa::path(
    delete,
    path = "/content/redirects/{id}",
    tag = "Admin",
    summary = "Delete a redirect",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_redirect(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    content_service(db, actor_id(&actor))
        .await?
        .delete_redirect(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_status_parsing_is_strict() {
        assert_eq!(
            parse_page_status("published").unwrap(),
            PageStatus::Published
        );
        assert_eq!(parse_page_status("draft").unwrap(), PageStatus::Draft);
        assert_eq!(parse_page_status("archived").unwrap(), PageStatus::Archived);
        assert!(parse_page_status("live").is_err());
    }

    #[test]
    fn page_list_query_defaults_to_live_pages() {
        let query: PageListQuery = serde_json::from_str("{}").expect("query");
        assert!(query.status.is_none());
        assert!(!query.include_deleted);
    }

    #[test]
    fn navigation_item_reorder_requires_ids() {
        let request: NavigationItemReorderRequest =
            serde_json::from_str(r#"{"item_ids": ["a", "b"]}"#).expect("request");
        assert_eq!(request.item_ids, vec!["a".to_owned(), "b".to_owned()]);
    }
}
