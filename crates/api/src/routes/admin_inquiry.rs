//! Admin inquiry form and inbox routes.
//!
//! Form editing requires `settings.*`; reading and triaging submissions
//! requires `customers.*`. Every mutation is audited.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult};
use fastcart_kernel::{ActorContext, AppError, InquiryFormId, InquiryId, Pagination};
use fastcart_module_inquiry::{
    CreateInquiryFormInput, Inquiry, InquiryFile, InquiryForm, InquiryFormField,
    InquiryFormRevision, InquiryFormService, InquiryFormSettings, InquiryListFilter,
    InquiryService, InquiryStatus, SaveInquiryDraftInput, UpdateInquiryFormInput,
    UpdateInquiryStatusInput,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the admin inquiry routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/inquiry-forms", get(list_forms).post(create_form))
        .route(
            "/inquiry-forms/{id}",
            get(get_form).patch(update_form).delete(archive_form),
        )
        .route("/inquiry-forms/{id}/draft", put(save_draft))
        .route("/inquiry-forms/{id}/publish", post(publish_form))
        .route(
            "/inquiry-forms/{id}/publish/{version}",
            post(publish_version),
        )
        .route("/inquiry-forms/{id}/revisions", get(list_revisions))
        .route("/inquiries", get(list_inquiries))
        .route("/inquiries/unread-count", get(unread_count))
        .route("/inquiries/{id}", get(get_inquiry).patch(update_status))
        .route("/inquiries/{id}/files/{file_id}", get(download_file))
}

/// Builds the documented admin inquiry routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_forms, create_form))
        .routes(routes!(get_form, update_form, archive_form))
        .routes(routes!(save_draft))
        .routes(routes!(publish_form))
        .routes(routes!(publish_version))
        .routes(routes!(list_revisions))
        .routes(routes!(list_inquiries))
        .routes(routes!(unread_count))
        .routes(routes!(get_inquiry, update_status))
        .routes(routes!(download_file))
}

/// Form creation request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateInquiryFormRequest {
    /// Display name.
    pub name: String,
    /// Optional stable handle.
    #[serde(default)]
    pub handle: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Form metadata update request.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct UpdateInquiryFormRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Draft save request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct SaveInquiryDraftRequest {
    /// Field definitions.
    pub fields: Vec<InquiryFormField>,
    /// Form settings.
    pub settings: InquiryFormSettings,
}

/// Publish request.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
pub struct PublishInquiryFormRequest {
    /// Optional revision version; defaults to the current draft.
    #[serde(default)]
    pub version: Option<i64>,
}

/// Form detail returned to the editor.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct InquiryFormDetail {
    /// Form identity.
    pub form: InquiryForm,
    /// Current draft revision, when one exists.
    pub draft: Option<InquiryFormRevision>,
    /// Currently published revision, when any.
    pub published_revision: Option<InquiryFormRevision>,
}

/// Inbox list query.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InquiryListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub form_id: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub from: Option<i64>,
    #[serde(default)]
    pub to: Option<i64>,
}

/// Inquiry status update request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateInquiryStatusRequest {
    pub status: String,
}

/// Inquiry detail returned to the inbox.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct InquiryDetail {
    /// Submission.
    pub inquiry: Inquiry,
    /// Form identity.
    pub form: InquiryForm,
    /// Revision used to validate the submission.
    pub revision: InquiryFormRevision,
    /// Attachments.
    pub files: Vec<InquiryFile>,
}

/// Unread counter.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct UnreadCountResponse {
    pub count: i64,
}

fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_user().map_err(ApiError::new)?;
    actor.require_permission(permission).map_err(ApiError::new)
}

#[utoipa::path(
    get,
    path = "/inquiry-forms",
    tag = "Admin",
    summary = "List inquiry forms",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = PageResponse<InquiryForm>))
)]
async fn list_forms(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<fastcart_http::PaginationQuery>,
) -> ApiResult<Json<PageResponse<InquiryForm>>> {
    authorize(&actor, "settings.read")?;
    let page = InquiryFormService::new(db)
        .list_forms(query.pagination()?)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/inquiry-forms",
    tag = "Admin",
    summary = "Create an inquiry form",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateInquiryFormRequest,
    responses((status = 201, body = InquiryForm))
)]
async fn create_form(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateInquiryFormRequest>,
) -> ApiResult<(StatusCode, Json<InquiryForm>)> {
    authorize(&actor, "settings.write")?;
    let form = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .create_form(CreateInquiryFormInput {
            name: request.name,
            handle: request.handle,
            description: request.description,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(form)))
}

#[utoipa::path(
    get,
    path = "/inquiry-forms/{id}",
    tag = "Admin",
    summary = "Get an inquiry form with its draft revision",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InquiryFormDetail))
)]
async fn get_form(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InquiryFormDetail>> {
    authorize(&actor, "settings.read")?;
    let id = InquiryFormId::from_string(id);
    let service = InquiryFormService::new(db);
    let form = service.get_form(&id).await?;
    let draft = service.get_draft(&id).await?;
    let published_revision = match form.published_version {
        Some(version) => Some(service.get_revision(&id, version).await?),
        None => None,
    };
    Ok(Json(InquiryFormDetail {
        form,
        draft,
        published_revision,
    }))
}

#[utoipa::path(
    patch,
    path = "/inquiry-forms/{id}",
    tag = "Admin",
    summary = "Update inquiry form metadata",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateInquiryFormRequest,
    responses((status = 200, body = InquiryForm))
)]
async fn update_form(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateInquiryFormRequest>,
) -> ApiResult<Json<InquiryForm>> {
    authorize(&actor, "settings.write")?;
    let form = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .update_form(
            &InquiryFormId::from_string(id),
            UpdateInquiryFormInput {
                name: request.name,
                description: request.description,
            },
        )
        .await?;
    Ok(Json(form))
}

#[utoipa::path(
    delete,
    path = "/inquiry-forms/{id}",
    tag = "Admin",
    summary = "Archive an inquiry form",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InquiryForm))
)]
async fn archive_form(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InquiryForm>> {
    authorize(&actor, "settings.write")?;
    let form = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .archive(&InquiryFormId::from_string(id))
        .await?;
    Ok(Json(form))
}

#[utoipa::path(
    put,
    path = "/inquiry-forms/{id}/draft",
    tag = "Admin",
    summary = "Save the draft revision",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = SaveInquiryDraftRequest,
    responses((status = 200, body = InquiryFormRevision))
)]
async fn save_draft(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<SaveInquiryDraftRequest>,
) -> ApiResult<Json<InquiryFormRevision>> {
    authorize(&actor, "settings.write")?;
    let revision = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .save_draft(
            &InquiryFormId::from_string(id),
            SaveInquiryDraftInput {
                fields: request.fields,
                settings: request.settings,
                created_by: actor.user_id.as_ref().map(ToString::to_string),
            },
        )
        .await?;
    Ok(Json(revision))
}

#[utoipa::path(
    post,
    path = "/inquiry-forms/{id}/publish",
    tag = "Admin",
    summary = "Publish an inquiry form revision",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = PublishInquiryFormRequest,
    responses((status = 200, body = InquiryForm))
)]
async fn publish_form(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<PublishInquiryFormRequest>,
) -> ApiResult<Json<InquiryForm>> {
    authorize(&actor, "settings.write")?;
    let form = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .publish(&InquiryFormId::from_string(id), request.version)
        .await?;
    Ok(Json(form))
}

#[utoipa::path(
    post,
    path = "/inquiry-forms/{id}/publish/{version}",
    tag = "Admin",
    summary = "Publish a specific inquiry form revision",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InquiryForm))
)]
async fn publish_version(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, version)): Path<(String, i64)>,
) -> ApiResult<Json<InquiryForm>> {
    authorize(&actor, "settings.write")?;
    let form = InquiryFormService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .publish(&InquiryFormId::from_string(id), Some(version))
        .await?;
    Ok(Json(form))
}

#[utoipa::path(
    get,
    path = "/inquiry-forms/{id}/revisions",
    tag = "Admin",
    summary = "List inquiry form revisions",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<InquiryFormRevision>))
)]
async fn list_revisions(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<InquiryFormRevision>>> {
    authorize(&actor, "settings.read")?;
    let revisions = InquiryFormService::new(db)
        .list_revisions(&InquiryFormId::from_string(id))
        .await?;
    Ok(Json(revisions))
}

#[utoipa::path(
    get,
    path = "/inquiries",
    tag = "Admin",
    summary = "List inquiries",
    security(("adminSession" = []), ("apiToken" = [])),
    params(InquiryListQuery),
    responses((status = 200, body = PageResponse<Inquiry>))
)]
async fn list_inquiries(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<InquiryListQuery>,
) -> ApiResult<Json<PageResponse<Inquiry>>> {
    authorize(&actor, "customers.read")?;
    let status = match query.status.as_deref() {
        Some(value) if !value.is_empty() => Some(
            InquiryStatus::parse(value)
                .ok_or_else(|| ApiError::new(AppError::validation("invalid inquiry status")))?,
        ),
        _ => None,
    };
    let filter = InquiryListFilter {
        status,
        form_id: query.form_id.map(InquiryFormId::from_string),
        query: query.query,
        from: query.from,
        to: query.to,
    };
    let pagination = Pagination::new(query.limit, query.offset).map_err(ApiError::new)?;
    let page = InquiryService::new(db).list(filter, pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/inquiries/unread-count",
    tag = "Admin",
    summary = "Count unread inquiries",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = UnreadCountResponse))
)]
async fn unread_count(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<UnreadCountResponse>> {
    authorize(&actor, "customers.read")?;
    let count = InquiryService::new(db).unread_count().await?;
    Ok(Json(UnreadCountResponse { count }))
}

#[utoipa::path(
    get,
    path = "/inquiries/{id}",
    tag = "Admin",
    summary = "Get an inquiry",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = InquiryDetail))
)]
async fn get_inquiry(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<InquiryDetail>> {
    authorize(&actor, "customers.read")?;
    let inquiry = InquiryService::new(db.clone())
        .get(&InquiryId::from_string(id))
        .await?;
    let form = InquiryFormService::new(db.clone())
        .get_form(&inquiry.form_id)
        .await?;
    let revision = InquiryFormService::new(db.clone())
        .get_revision(&inquiry.form_id, inquiry.form_version)
        .await?;
    let files = InquiryService::new(db).files_for(&inquiry.id).await?;
    Ok(Json(InquiryDetail {
        inquiry,
        form,
        revision,
        files,
    }))
}

#[utoipa::path(
    patch,
    path = "/inquiries/{id}",
    tag = "Admin",
    summary = "Update an inquiry status",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateInquiryStatusRequest,
    responses((status = 200, body = Inquiry))
)]
async fn update_status(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateInquiryStatusRequest>,
) -> ApiResult<Json<Inquiry>> {
    authorize(&actor, "customers.write")?;
    let status = InquiryStatus::parse(&request.status)
        .ok_or_else(|| ApiError::new(AppError::validation("invalid inquiry status")))?;
    let inquiry = InquiryService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .update_status(
            &InquiryId::from_string(id),
            UpdateInquiryStatusInput { status },
        )
        .await?;
    Ok(Json(inquiry))
}

#[utoipa::path(
    get,
    path = "/inquiries/{id}/files/{file_id}",
    tag = "Admin",
    summary = "Download an inquiry attachment",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, content_type = "application/octet-stream"))
)]
async fn download_file(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, file_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    authorize(&actor, "customers.read")?;
    let file = InquiryService::new(db)
        .file_for_download(
            &InquiryId::from_string(id),
            &fastcart_kernel::InquiryFileId::from_string(file_id),
        )
        .await?;
    let bytes = context
        .storage
        .get(&file.storage_key)
        .await
        .map_err(|error| ApiError::new(AppError::from(error)))?;
    let filename = sanitize_header_file_name(file.original_name.as_deref().unwrap_or("attachment"));
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&file.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn sanitize_header_file_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or("attachment")
        .chars()
        .filter(|character| {
            !character.is_control() && !matches!(character, '"' | '\\' | '\r' | '\n')
        })
        .take(255)
        .collect()
}
