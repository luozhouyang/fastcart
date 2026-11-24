use super::*;

#[utoipa::path(
    get,
    path = "/shipping/profiles",
    tag = "Admin",
    summary = "List shipping profiles",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<ShippingProfile>))
)]
pub(super) async fn list_profiles(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ShippingProfile>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ShippingService::new(db).list_profiles(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/shipping/profiles",
    tag = "Admin",
    summary = "Create a shipping profile",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateShippingProfileRequest,
    responses((status = 201, body = ShippingProfile))
)]
pub(super) async fn create_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateShippingProfileRequest>,
) -> ApiResult<(StatusCode, Json<ShippingProfile>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let profile = ShippingService::new(db)
        .create_profile(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(profile)))
}

#[utoipa::path(
    get,
    path = "/shipping/profiles/{id}",
    tag = "Admin",
    summary = "Get a shipping profile",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ShippingProfile))
)]
pub(super) async fn get_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingProfile>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    let profile = ShippingService::new(db).get_profile(&id).await?;
    Ok(Json(profile))
}

#[utoipa::path(
    patch,
    path = "/shipping/profiles/{id}",
    tag = "Admin",
    summary = "Update a shipping profile",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateShippingProfileRequest,
    responses((status = 200, body = ShippingProfile))
)]
pub(super) async fn update_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingProfileRequest>,
) -> ApiResult<Json<ShippingProfile>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    let profile = ShippingService::new(db)
        .update_profile(&id, request.into_input())
        .await?;
    Ok(Json(profile))
}

#[utoipa::path(
    delete,
    path = "/shipping/profiles/{id}",
    tag = "Admin",
    summary = "Delete a shipping profile",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn delete_profile(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingProfileId::from_string(id);
    ShippingService::new(db).delete_profile(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
