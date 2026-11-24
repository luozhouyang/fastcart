use super::*;

#[utoipa::path(
    get,
    path = "/shipping/fulfillment-sets",
    tag = "Admin",
    summary = "List fulfillment sets",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<FulfillmentSet>))
)]
pub(super) async fn list_fulfillment_sets(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<FulfillmentSet>>> {
    require_permission(&actor, READ_PERMISSION)?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = ShippingService::new(db)
        .list_fulfillment_sets(pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/shipping/fulfillment-sets",
    tag = "Admin",
    summary = "Create a fulfillment set",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateFulfillmentSetRequest,
    responses((status = 201, body = FulfillmentSet))
)]
pub(super) async fn create_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateFulfillmentSetRequest>,
) -> ApiResult<(StatusCode, Json<FulfillmentSet>)> {
    require_permission(&actor, WRITE_PERMISSION)?;
    let set = ShippingService::new(db)
        .create_fulfillment_set(request.into_input())
        .await?;
    Ok((StatusCode::CREATED, Json(set)))
}

#[utoipa::path(
    get,
    path = "/shipping/fulfillment-sets/{id}",
    tag = "Admin",
    summary = "Get a fulfillment set",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = FulfillmentSet))
)]
pub(super) async fn get_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<FulfillmentSet>> {
    require_permission(&actor, READ_PERMISSION)?;
    Ok(Json(
        ShippingService::new(db).get_fulfillment_set(&id).await?,
    ))
}

#[utoipa::path(
    patch,
    path = "/shipping/fulfillment-sets/{id}",
    tag = "Admin",
    summary = "Update a fulfillment set",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateFulfillmentSetRequest,
    responses((status = 200, body = FulfillmentSet))
)]
pub(super) async fn update_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateFulfillmentSetRequest>,
) -> ApiResult<Json<FulfillmentSet>> {
    require_permission(&actor, WRITE_PERMISSION)?;
    Ok(Json(
        ShippingService::new(db)
            .update_fulfillment_set(&id, request.into_input())
            .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/shipping/fulfillment-sets/{id}",
    tag = "Admin",
    summary = "Delete a fulfillment set",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204, description = "No content"))
)]
pub(super) async fn delete_fulfillment_set(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    require_permission(&actor, WRITE_PERMISSION)?;
    ShippingService::new(db).delete_fulfillment_set(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
