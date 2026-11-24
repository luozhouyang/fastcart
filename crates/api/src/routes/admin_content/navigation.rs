use super::*;

#[utoipa::path(
    get,
    path = "/content/navigations",
    tag = "Admin",
    summary = "List navigations",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Navigation>))
)]
pub(super) async fn list_navigations(
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
pub(super) async fn create_navigation(
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
pub(super) async fn get_navigation(
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
pub(super) async fn update_navigation(
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
pub(super) async fn delete_navigation(
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
pub(super) async fn get_navigation_tree(
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
pub(super) async fn add_navigation_item(
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
pub(super) async fn update_navigation_item(
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
pub(super) async fn delete_navigation_item(
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
pub(super) async fn reorder_navigation_items(
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
