use super::*;

#[utoipa::path(
    get,
    path = "/content/pages",
    tag = "Admin",
    summary = "List content pages",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, PageListQuery),
    responses((status = 200, body = PageResponse<Page>))
)]
pub(super) async fn list_pages(
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
pub(super) async fn create_page(
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
pub(super) async fn get_page(
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
pub(super) async fn update_page(
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
pub(super) async fn delete_page(
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
pub(super) async fn publish_page(
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
pub(super) async fn unpublish_page(
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
pub(super) async fn archive_page(
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

#[utoipa::path(
    get,
    path = "/content/pages/{id}/versions",
    tag = "Admin",
    summary = "List page versions",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<PageVersion>))
)]
pub(super) async fn list_page_versions(
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

#[utoipa::path(
    post,
    path = "/content/pages/{id}/versions/{version}/restore",
    tag = "Admin",
    summary = "Restore a page version",
    security(("adminSession" = []), ("apiToken" = [])),
    params(RestorePageVersionQuery),
    responses((status = 200, body = Page))
)]
pub(super) async fn restore_page_version(
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
