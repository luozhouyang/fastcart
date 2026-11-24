use super::*;

#[utoipa::path(
    get,
    path = "/shipping/options",
    tag = "Admin",
    summary = "List shipping options",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, ListOptionsQuery),
    responses((status = 200, body = PageResponse<ShippingOption>))
)]
pub(super) async fn list_options(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<ListOptionsQuery>,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ShippingOption>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let service = ShippingService::new(db);
    let page = match query.profile_id {
        Some(profile_id) => {
            let profile_id = ShippingProfileId::from_string(profile_id);
            service
                .list_options_for_profile(&profile_id, pagination)
                .await?
        }
        None => service.list_options(pagination).await?,
    };
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/shipping/options",
    tag = "Admin",
    summary = "Create a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateShippingOptionRequest,
    responses((status = 201, body = ShippingOption))
)]
pub(super) async fn create_option(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateShippingOptionRequest>,
) -> ApiResult<(StatusCode, Json<ShippingOption>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let input = request.into_input()?;
    let provider_id = input.provider_id.as_deref().unwrap_or("manual");
    ensure_provider(&context, provider_id)?;
    let option = ShippingService::new(db).create_option(input).await?;
    Ok((StatusCode::CREATED, Json(option)))
}

#[utoipa::path(
    get,
    path = "/shipping/options/{id}",
    tag = "Admin",
    summary = "Get a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ShippingOption))
)]
pub(super) async fn get_option(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingOption>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let option = ShippingService::new(db).get_option(&id).await?;
    Ok(Json(option))
}

#[utoipa::path(
    patch,
    path = "/shipping/options/{id}",
    tag = "Admin",
    summary = "Update a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateShippingOptionRequest,
    responses((status = 200, body = ShippingOption))
)]
pub(super) async fn update_option(
    State(context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingOptionRequest>,
) -> ApiResult<Json<ShippingOption>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let input = request.into_input()?;
    if let Some(provider_id) = input.provider_id.as_deref() {
        ensure_provider(&context, provider_id)?;
    }
    let option = ShippingService::new(db).update_option(&id, input).await?;
    Ok(Json(option))
}

#[utoipa::path(
    delete,
    path = "/shipping/options/{id}",
    tag = "Admin",
    summary = "Delete a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn delete_option(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db).delete_option(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/shipping/options/{id}/locations",
    tag = "Admin",
    summary = "Assign a location to a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = AddLocationRequest,
    responses((status = 204, description = "No content"))
)]
pub(super) async fn add_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<AddLocationRequest>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db)
        .assign_location(&id, &request.location_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/shipping/options/{id}/locations/{location_id}",
    tag = "Admin",
    summary = "Remove a location from a shipping option",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn remove_location(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((id, location_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    ShippingService::new(db)
        .remove_location(&id, &location_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/shipping/options-list",
    tag = "Admin",
    summary = "List shipping options for a destination",
    security(("adminSession" = []), ("apiToken" = [])),
    params(DestinationOptionsQuery),
    responses((status = 200, body = Vec<ResolvedShippingOption>))
)]
pub(super) async fn list_options_for_destination(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiQuery(query): ApiQuery<DestinationOptionsQuery>,
) -> ApiResult<Json<Vec<ResolvedShippingOption>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let country = CountryCode::new(query.country).map_err(AppError::from)?;
    let options = ShippingService::new(db)
        .list_options_for(&country, query.subtotal.unwrap_or(0))
        .await?;
    Ok(Json(options))
}

#[utoipa::path(
    get,
    path = "/shipping/options/{id}/rate",
    tag = "Admin",
    summary = "Select a shipping rate for a destination",
    security(("adminSession" = []), ("apiToken" = [])),
    params(SelectRateQuery),
    responses((status = 200, body = Option<SelectedShippingRate>))
)]
pub(super) async fn select_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<SelectRateQuery>,
) -> ApiResult<Json<Option<SelectedShippingRate>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let country = CountryCode::new(query.country).map_err(AppError::from)?;
    let rate = ShippingService::new(db)
        .select_rate(
            &id,
            &country,
            query.province.as_deref(),
            query.subtotal.unwrap_or(0),
        )
        .await?;
    Ok(Json(rate))
}
