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
use fastcart_storefront::{PageThemeRef, SUPPORTED_EDITOR_SCHEMA_VERSION};
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

mod navigation;
mod pages;
mod preview;
mod redirects;

use navigation::*;
use pages::*;
use preview::*;
use redirects::*;

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

/// Navigation creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateNavigationRequest {
    name: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    metadata: Metadata,
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

/// Redirect creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateRedirectRequest {
    from_path: String,
    to_path: String,
    #[serde(default)]
    status_code: Option<i64>,
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

/// Body of a preview-token revocation request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct RevokePreviewTokenRequest {
    /// Identifier returned when the token was issued.
    pub jti: String,
}

/// Optional optimistic revision for a page restore.
#[derive(Clone, Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct RestorePageVersionQuery {
    /// Revision the caller last observed.
    #[serde(default)]
    base_revision: Option<i64>,
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

impl From<CreateNavigationRequest> for CreateNavigationInput {
    fn from(request: CreateNavigationRequest) -> Self {
        Self {
            name: request.name,
            handle: request.handle,
            metadata: request.metadata,
        }
    }
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
/// Page bodies are stamped with a [`PageThemeRef`] built from the active theme
/// metadata. Themes are deployment artifacts (ADR 0008), so the registry holds
/// no editor contract to validate against; a generic editor arrives later. The
/// actor is recorded by the service inside each mutation transaction, so
/// content audits are atomic with the change and never need a second write.
async fn content_service(db: StoreDatabase, actor_id: Option<String>) -> ApiResult<ContentService> {
    let theme_ref = match ThemeService::new(db.clone()).active().await? {
        Some(theme) => Some(PageThemeRef {
            id: theme.manifest.id,
            version: theme.manifest.version,
            schema_version: SUPPORTED_EDITOR_SCHEMA_VERSION,
        }),
        None => None,
    };
    Ok(ContentService::new(db)
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
