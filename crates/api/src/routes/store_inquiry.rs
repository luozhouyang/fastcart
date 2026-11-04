//! Public storefront inquiry routes.
//!
//! Forms expose only their published revision. Submissions are rate limited,
//! validated against the revision schema and recorded together with an outbox
//! event. Attachments use a two-phase upload: bytes are stored privately first
//! and claimed by the submission transaction.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use fastcart_http::{ApiError, ApiJson, ApiQuery, ApiResult, StoreCtx};
use fastcart_kernel::{AppError, InquiryFileId, now_timestamp};
use fastcart_module_inquiry::{
    CreateInquiryInput, InquiryFileService, InquiryFormField, InquiryFormService, InquiryService,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::state::AppContext;

/// Builds the public inquiry routes.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/inquiry-forms/{handle}", get(get_form))
        .route("/inquiries", post(create_inquiry))
        .route("/inquiry-files", post(upload_file))
}

/// Builds the documented public inquiry routes.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(get_form))
        .routes(routes!(create_inquiry))
}

/// Public form shape. Internal notification settings are never exposed.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct PublicInquiryForm {
    /// Stable form handle.
    pub handle: String,
    /// Published revision version.
    pub version: i64,
    /// Field definitions in display order.
    pub fields: Vec<InquiryFormField>,
    /// Public presentation settings.
    pub settings: PublicInquiryFormSettings,
}

/// Public presentation settings.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct PublicInquiryFormSettings {
    /// Submit button label.
    pub submit_label: String,
    /// Message shown after a successful submission.
    pub success_message: String,
    /// Optional post-submit redirect.
    pub redirect_url: Option<String>,
    /// Optional consent copy.
    pub consent_text: Option<String>,
    /// Whether consent is mandatory.
    pub consent_required: bool,
    /// Whether the form accepts attachments.
    pub allow_attachments: bool,
}

/// Envelope for the public form response.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct PublicInquiryFormResponse {
    /// Published form.
    pub form: PublicInquiryForm,
}

/// Submission request.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInquiryRequest {
    /// Published form handle.
    pub form_handle: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub files: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub consent: bool,
}

/// Submission response.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CreateInquiryResponse {
    /// Submission identifier.
    pub id: String,
    /// Lifecycle status.
    pub status: String,
}

/// Upload query.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// Published form handle.
    pub form_handle: String,
    /// File field key.
    pub field_key: String,
}

/// Upload response.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct UploadFileResponse {
    /// Pending file identifier.
    pub id: String,
    /// Expiry timestamp.
    pub expires_at: i64,
}

/// Returns one published form.
#[utoipa::path(
    get,
    path = "/inquiry-forms/{handle}",
    tag = "Store",
    summary = "Get a published inquiry form",
    responses((status = 200, body = PublicInquiryFormResponse))
)]
async fn get_form(
    StoreDb(db): StoreDb,
    Path(handle): Path<String>,
) -> ApiResult<axum::Json<PublicInquiryFormResponse>> {
    let published = InquiryFormService::new(db)
        .published_by_handle(&handle)
        .await?;
    let settings = &published.revision.settings;
    Ok(axum::Json(PublicInquiryFormResponse {
        form: PublicInquiryForm {
            handle: published.form.handle,
            version: published
                .form
                .published_version
                .unwrap_or(published.revision.version),
            fields: published.revision.fields,
            settings: PublicInquiryFormSettings {
                submit_label: settings.submit_label.clone(),
                success_message: settings.success_message.clone(),
                redirect_url: settings.redirect_url.clone(),
                consent_text: settings.consent_text.clone(),
                consent_required: settings.consent_required,
                allow_attachments: settings.allow_attachments,
            },
        },
    }))
}

/// Accepts one inquiry submission.
#[utoipa::path(
    post,
    path = "/inquiries",
    tag = "Store",
    summary = "Submit an inquiry",
    request_body = CreateInquiryRequest,
    responses((status = 201, body = CreateInquiryResponse))
)]
async fn create_inquiry(
    State(context): State<Arc<AppContext>>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    headers: HeaderMap,
    ApiJson(request): ApiJson<CreateInquiryRequest>,
) -> ApiResult<(StatusCode, axum::Json<CreateInquiryResponse>)> {
    let address = client_address(&headers);
    if !context
        .inquiry_limiter
        .check(&format!("{}:{address}", store.store_id))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let files = request
        .files
        .into_iter()
        .map(|(field, ids)| {
            (
                field,
                ids.into_iter()
                    .map(InquiryFileId::from_string)
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    let inquiry = InquiryService::new(db)
        .create(CreateInquiryInput {
            form_handle: request.form_handle,
            name: request.name,
            email: request.email,
            phone: request.phone,
            subject: request.subject,
            message: request.message,
            data: request.data,
            files,
            source: request.source,
            page_url: request.page_url,
            locale: request.locale,
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            ip_hash: Some(hash_client_address(&store.store_id, &address)),
            idempotency_key,
            consent: request.consent,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(CreateInquiryResponse {
            id: inquiry.id.to_string(),
            status: inquiry.status.as_str().to_owned(),
        }),
    ))
}

/// Stores one pending attachment.
#[utoipa::path(
    post,
    path = "/inquiry-files",
    tag = "Store",
    summary = "Upload an inquiry attachment",
    params(UploadQuery),
    responses((status = 201, body = UploadFileResponse))
)]
async fn upload_file(
    State(context): State<Arc<AppContext>>,
    StoreCtx(store): StoreCtx,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<UploadQuery>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, axum::Json<UploadFileResponse>)> {
    let address = client_address(&headers);
    if !context
        .upload_limiter
        .check(&format!("{}:{address}", store.store_id))
    {
        return Err(ApiError::new(AppError::RateLimited));
    }

    let published = InquiryFormService::new(db.clone())
        .published_by_handle(&query.form_handle)
        .await?;
    let field = published
        .revision
        .fields
        .iter()
        .find(|field| field.key == query.field_key)
        .ok_or_else(|| {
            ApiError::new(AppError::validation(format!(
                "unknown file field: {}",
                query.field_key
            )))
        })?;
    let file_config = field.file.as_ref().ok_or_else(|| {
        ApiError::new(AppError::validation(format!(
            "field {} does not accept files",
            query.field_key
        )))
    })?;
    if !published.revision.settings.allow_attachments {
        return Err(ApiError::new(AppError::validation(
            "this form does not accept attachments",
        )));
    }

    let mut uploaded = None;
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        ApiError::new(AppError::validation(format!(
            "invalid multipart body: {error}"
        )))
    })? {
        if field.name() != Some("file") {
            continue;
        }
        let original_name = field
            .file_name()
            .map(sanitize_file_name)
            .filter(|name| !name.is_empty());
        let bytes = field.bytes().await.map_err(|error| {
            ApiError::new(AppError::validation(format!(
                "invalid upload body: {error}"
            )))
        })?;
        if bytes.is_empty() {
            return Err(ApiError::new(AppError::validation(
                "uploaded file must not be empty",
            )));
        }
        if bytes.len() > context.config.max_upload_bytes {
            return Err(ApiError::new(AppError::validation(format!(
                "uploaded file must not exceed {} bytes",
                context.config.max_upload_bytes
            ))));
        }
        let Some(content_type) = sniff_content_type(&bytes) else {
            return Err(ApiError::new(AppError::validation("unsupported file type")));
        };
        if !file_config
            .accept
            .iter()
            .any(|allowed| allowed == content_type)
        {
            return Err(ApiError::new(AppError::validation(format!(
                "file type {content_type} is not accepted by this field"
            ))));
        }
        if bytes.len() as u64 > file_config.max_bytes {
            return Err(ApiError::new(AppError::validation(format!(
                "file must not exceed {} bytes",
                file_config.max_bytes
            ))));
        }
        let id = fastcart_kernel::InquiryFileId::new();
        let now = now_timestamp();
        let storage_key = format!(
            "inquiries/{}/{}/{}/{}",
            store.store_id,
            published.form.id,
            &id.as_str()[..id.as_str().len().min(16)],
            uuid::Uuid::now_v7().simple()
        );
        let checksum = hex::encode(Sha256::digest(&bytes));
        let stored = context
            .storage
            .put(&storage_key, &bytes, Some(content_type))
            .await
            .map_err(|error| ApiError::new(AppError::from(error)))?;
        let expires_at = now.saturating_add(context.config.upload_ttl_seconds);
        let file = match InquiryFileService::new(db.clone())
            .create_pending(
                &query.form_handle,
                &query.field_key,
                stored.key,
                original_name,
                content_type.to_owned(),
                i64::try_from(bytes.len()).unwrap_or(i64::MAX),
                checksum,
                expires_at,
            )
            .await
        {
            Ok(file) => file,
            Err(error) => {
                let _ = context.storage.delete(&storage_key).await;
                return Err(ApiError::new(error));
            }
        };
        uploaded = Some(file);
        break;
    }

    let Some(file) = uploaded else {
        return Err(ApiError::new(AppError::validation(
            "multipart field 'file' is required",
        )));
    };
    Ok((
        StatusCode::CREATED,
        axum::Json(UploadFileResponse {
            id: file.id.to_string(),
            expires_at: file.expires_at.unwrap_or_else(now_timestamp),
        }),
    ))
}

fn client_address(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').next().unwrap_or_default().trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn hash_client_address(store_id: &fastcart_kernel::StoreId, address: &str) -> String {
    hex::encode(Sha256::digest(format!("{store_id}:{address}").as_bytes()))
}

fn sanitize_file_name(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .chars()
        .filter(|character| !character.is_control())
        .take(255)
        .collect()
}

fn sniff_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"%PDF-") {
        return Some("application/pdf");
    }
    None
}
