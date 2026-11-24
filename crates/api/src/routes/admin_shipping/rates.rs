use super::*;

#[utoipa::path(
    get,
    path = "/shipping/options/{id}/rates",
    tag = "Admin",
    summary = "List shipping rates",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<ShippingRate>))
)]
pub(super) async fn list_rates(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ShippingRate>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let id = ShippingOptionId::from_string(id);
    let rates = ShippingService::new(db).list_rates(&id).await?;
    Ok(Json(rates))
}

#[utoipa::path(
    post,
    path = "/shipping/options/{id}/rates",
    tag = "Admin",
    summary = "Create a shipping rate",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateShippingRateRequest,
    responses((status = 201, body = ShippingRate))
)]
pub(super) async fn create_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateShippingRateRequest>,
) -> ApiResult<(StatusCode, Json<ShippingRate>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let input = request.into_input(ShippingOptionId::from_string(id))?;
    let rate = ShippingService::new(db).create_rate(input).await?;
    Ok((StatusCode::CREATED, Json(rate)))
}

#[utoipa::path(
    get,
    path = "/shipping/rates/{id}",
    tag = "Admin",
    summary = "Get a shipping rate",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ShippingRate))
)]
pub(super) async fn get_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ShippingRate>> {
    require_permission(&actor, READ_PERMISSION)?;
    let rate = ShippingService::new(db).get_rate(&id).await?;
    Ok(Json(rate))
}

#[utoipa::path(
    patch,
    path = "/shipping/rates/{id}",
    tag = "Admin",
    summary = "Update a shipping rate",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateShippingRateRequest,
    responses((status = 200, body = ShippingRate))
)]
pub(super) async fn update_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateShippingRateRequest>,
) -> ApiResult<Json<ShippingRate>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let rate = ShippingService::new(db)
        .update_rate(&id, request.into_input()?)
        .await?;
    Ok(Json(rate))
}

#[utoipa::path(
    delete,
    path = "/shipping/rates/{id}",
    tag = "Admin",
    summary = "Delete a shipping rate",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn delete_rate(
    State(_context): State<Arc<AppContext>>,
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    ShippingService::new(db).delete_rate(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
