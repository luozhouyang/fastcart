use super::*;

/// Tag creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct TagRequest {
    value: String,
}

/// Body linking an existing tag to a product.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ProductTagRequest {
    tag_id: String,
}

/// Resolves a tag identifier to its value.
///
/// The catalog service attaches tags by value only, so the identifier is
/// resolved by scanning the tag list.
async fn resolve_tag_value(service: &CatalogService, tag_id: &str) -> ApiResult<String> {
    let mut offset = 0_i64;
    loop {
        let page = service.list_tags(Pagination { limit: 100, offset }).await?;
        if let Some(tag) = page.data.iter().find(|tag| tag.id == tag_id) {
            return Ok(tag.value.clone());
        }
        offset += 100;
        if offset >= page.count {
            return Err(ApiError::new(AppError::not_found("tag", tag_id)));
        }
    }
}

#[utoipa::path(
    get,
    path = "/catalog/tags",
    tag = "Admin",
    summary = "List tags",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Tag>))
)]
pub(super) async fn list_tags(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Tag>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db).list_tags(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/catalog/tags",
    tag = "Admin",
    summary = "Create a tag",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = TagRequest,
    responses((status = 201, body = Tag))
)]
pub(super) async fn create_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<TagRequest>,
) -> ApiResult<(StatusCode, Json<Tag>)> {
    authorize(&actor, "products.write")?;
    let tag = CatalogService::new(db).create_tag(&request.value).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/tags",
    tag = "Admin",
    summary = "Attach a tag to a product",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ProductTagRequest,
    responses((status = 200, body = Tag))
)]
pub(super) async fn attach_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<ProductTagRequest>,
) -> ApiResult<Json<Tag>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let service = CatalogService::new(db);
    let value = resolve_tag_value(&service, &request.tag_id).await?;
    let tag = service.attach_tag(product_id.as_str(), &value).await?;
    Ok(Json(tag))
}

#[utoipa::path(
    delete,
    path = "/catalog/products/{id}/tags/{tag_id}",
    tag = "Admin",
    summary = "Detach a tag from a product",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn detach_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((product_id, tag_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(product_id);
    let tag_id = TagId::from_string(tag_id);
    CatalogService::new(db)
        .detach_tag(product_id.as_str(), tag_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
