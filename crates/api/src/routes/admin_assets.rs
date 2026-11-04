//! Admin media library routes.
//!
//! Asset bytes are validated by magic-byte sniffing, stored through the
//! provider-independent [`fastcart_infrastructure::Storage`] adapter, and
//! referenced by metadata only. The public serving endpoint lives in
//! [`super::store_assets`].

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiResult};
use fastcart_kernel::{ActorContext, AppError, Pagination};
use fastcart_module_asset::{Asset, AssetFilter, AssetService, CreateAssetInput, MAX_SIZE_BYTES};
use serde::Deserialize;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::store_inquiry::{sanitize_file_name, sniff_content_type};
use super::{StoreDb, record_store_audit};
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/assets", get(list_assets).post(upload_asset))
        .route("/assets/{id}", get(get_asset).delete(delete_asset))
}

/// Builds the documented admin media routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_assets))
        .routes(routes!(upload_asset))
        .routes(routes!(get_asset, delete_asset))
}

/// Query parameters for the media library.
#[derive(Clone, Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
struct AssetListQuery {
    /// Case-insensitive filename or alt-text search.
    #[serde(default)]
    search: Option<String>,
    /// Page size.
    #[serde(default)]
    limit: Option<i64>,
    /// Page offset.
    #[serde(default)]
    offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/assets",
    tag = "Admin",
    summary = "List media assets",
    security(("adminSession" = []), ("apiToken" = [])),
    params(AssetListQuery),
    responses((status = 200, body = PageResponse<Asset>))
)]
async fn list_assets(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<AssetListQuery>,
) -> ApiResult<Json<PageResponse<Asset>>> {
    authorize(&actor, "themes.read")?;
    let pagination = Pagination::new(query.limit, query.offset).map_err(ApiError::new)?;
    let page = AssetService::new(db)
        .list_assets(
            AssetFilter {
                search: query.search,
            },
            pagination,
        )
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    get,
    path = "/assets/{id}",
    tag = "Admin",
    summary = "Get a media asset",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Asset))
)]
async fn get_asset(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Asset>> {
    authorize(&actor, "themes.read")?;
    let asset = AssetService::new(db).get_asset(&id).await?;
    Ok(Json(asset))
}

#[utoipa::path(
    post,
    path = "/assets",
    tag = "Admin",
    summary = "Upload a media asset",
    description = "Multipart upload with a `file` part and optional `alt`, `width` and `height` text parts. Content type is detected from the bytes.",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 201, body = Asset))
)]
async fn upload_asset(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    mut multipart: Multipart,
) -> ApiResult<(StatusCode, Json<Asset>)> {
    authorize(&actor, "themes.write")?;

    let mut file: Option<(String, Vec<u8>, Option<String>)> = None;
    let mut alt: Option<String> = None;
    let mut width: Option<i64> = None;
    let mut height: Option<i64> = None;

    while let Some(field) = multipart.next_field().await.map_err(|error| {
        ApiError::new(AppError::validation(format!(
            "invalid multipart body: {error}"
        )))
    })? {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "file" => {
                let filename = field
                    .file_name()
                    .map(sanitize_file_name)
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| "file".to_owned());
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
                if bytes.len() > context.config.max_upload_bytes
                    || i64::try_from(bytes.len()).unwrap_or(i64::MAX) > MAX_SIZE_BYTES
                {
                    return Err(ApiError::new(AppError::validation(format!(
                        "uploaded file must not exceed {} bytes",
                        context.config.max_upload_bytes.min(MAX_SIZE_BYTES as usize)
                    ))));
                }
                let Some(content_type) = sniff_content_type(&bytes) else {
                    return Err(ApiError::new(AppError::validation("unsupported file type")));
                };
                file = Some((filename, bytes.to_vec(), Some(content_type.to_owned())));
            }
            "alt" => {
                alt = Some(text_field(field).await?);
            }
            "width" => width = Some(number_field(field, "width").await?),
            "height" => height = Some(number_field(field, "height").await?),
            _ => {}
        }
    }

    let (filename, bytes, content_type) = file
        .ok_or_else(|| ApiError::new(AppError::validation("multipart field `file` is required")))?;
    let content_type = content_type.expect("set when the file part is read");
    let asset_id = uuid::Uuid::now_v7().simple().to_string();
    let storage_key = format!(
        "assets/{}/{}/{}",
        db.store_id().as_str(),
        asset_id,
        storage_name(&filename)
    );
    context
        .storage
        .put(&storage_key, &bytes, Some(&content_type))
        .await
        .map_err(|error| AppError::internal(format!("could not store asset: {error}")))?;

    let input = CreateAssetInput {
        filename,
        content_type,
        size_bytes: i64::try_from(bytes.len()).unwrap_or(i64::MAX),
        storage_key: storage_key.clone(),
        alt: alt.filter(|value| !value.trim().is_empty()),
        width,
        height,
        created_by: actor.user_id.as_ref().map(ToString::to_string),
    };
    let asset = match AssetService::new(db).create_asset(input).await {
        Ok(asset) => asset,
        Err(error) => {
            // Do not leave an unreferenced blob behind when metadata validation
            // fails (for example an unexpected content type or size).
            let _ = context.storage.delete(&storage_key).await;
            return Err(ApiError::new(error));
        }
    };
    record_store_audit(
        &context,
        &actor,
        "asset.created",
        "asset",
        &asset.id,
        serde_json::json!({
            "filename": &asset.filename,
            "content_type": &asset.content_type,
            "size_bytes": asset.size_bytes,
        }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(asset)))
}

#[utoipa::path(
    delete,
    path = "/assets/{id}",
    tag = "Admin",
    summary = "Delete a media asset",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
async fn delete_asset(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "themes.write")?;
    let asset = AssetService::new(db).delete_asset(&id).await?;
    if let Err(error) = context.storage.delete(&asset.storage_key).await {
        tracing::warn!(asset_id = %asset.id, error = %error, "could not delete asset object");
    }
    record_store_audit(
        &context,
        &actor,
        "asset.deleted",
        "asset",
        &asset.id,
        serde_json::json!({ "filename": asset.filename }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// Reads a text multipart field.
async fn text_field(field: axum::extract::multipart::Field<'_>) -> ApiResult<String> {
    let bytes = field.bytes().await.map_err(|error| {
        ApiError::new(AppError::validation(format!(
            "invalid multipart field: {error}"
        )))
    })?;
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::new(AppError::validation("multipart field must be UTF-8")))
}

/// Reads a non-negative integer multipart field.
async fn number_field(field: axum::extract::multipart::Field<'_>, name: &str) -> ApiResult<i64> {
    let value = text_field(field).await?;
    value
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|number| *number >= 0)
        .ok_or_else(|| ApiError::new(AppError::validation(format!("{name} must be non-negative"))))
}

/// Builds a storage-safe file name component.
fn storage_name(filename: &str) -> String {
    let name: String = filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "file".to_owned()
    } else {
        name
    }
}

/// Enforces a permission at the handler boundary.
fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}
