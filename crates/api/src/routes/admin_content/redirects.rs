use super::*;

#[utoipa::path(
    get,
    path = "/content/redirects",
    tag = "Admin",
    summary = "List redirects",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Redirect>))
)]
pub(super) async fn list_redirects(
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
pub(super) async fn create_redirect(
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
pub(super) async fn get_redirect(
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
pub(super) async fn delete_redirect(
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
