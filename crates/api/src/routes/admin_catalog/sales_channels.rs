use super::*;

/// Sales-channel creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateSalesChannelRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateSalesChannelRequest> for CreateSalesChannelInput {
    fn from(request: CreateSalesChannelRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            metadata: request.metadata,
        }
    }
}

/// Partial sales-channel update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateSalesChannelRequest {
    name: Option<String>,
    description: Option<String>,
    metadata: Option<Metadata>,
}

impl From<UpdateSalesChannelRequest> for UpdateSalesChannelInput {
    fn from(request: UpdateSalesChannelRequest) -> Self {
        Self {
            name: request.name,
            description: request.description,
            metadata: request.metadata,
        }
    }
}

#[utoipa::path(
    get,
    path = "/sales-channels",
    tag = "Admin",
    summary = "List sales channels",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<SalesChannel>))
)]
pub(super) async fn list_sales_channels(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<SalesChannel>>> {
    authorize(&actor, "settings.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = StoreService::new(db)
        .list_sales_channels(pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/sales-channels",
    tag = "Admin",
    summary = "Create a sales channel",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateSalesChannelRequest,
    responses((status = 201, body = SalesChannel))
)]
pub(super) async fn create_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateSalesChannelRequest>,
) -> ApiResult<(StatusCode, Json<SalesChannel>)> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db)
        .create_sales_channel(CreateSalesChannelInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(channel)))
}

#[utoipa::path(
    get,
    path = "/sales-channels/{id}",
    tag = "Admin",
    summary = "Get a sales channel",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = SalesChannel))
)]
pub(super) async fn get_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.read")?;
    let channel = StoreService::new(db).get_sales_channel(&id).await?;
    Ok(Json(channel))
}

#[utoipa::path(
    patch,
    path = "/sales-channels/{id}",
    tag = "Admin",
    summary = "Update a sales channel",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateSalesChannelRequest,
    responses((status = 200, body = SalesChannel))
)]
pub(super) async fn update_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateSalesChannelRequest>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db)
        .update_sales_channel(&id, request.into())
        .await?;
    Ok(Json(channel))
}

#[utoipa::path(
    post,
    path = "/sales-channels/{id}/disable",
    tag = "Admin",
    summary = "Disable a sales channel",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = SalesChannel))
)]
pub(super) async fn disable_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db).disable_sales_channel(&id).await?;
    Ok(Json(channel))
}

#[utoipa::path(
    post,
    path = "/sales-channels/{id}/enable",
    tag = "Admin",
    summary = "Enable a sales channel",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = SalesChannel))
)]
pub(super) async fn enable_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db).enable_sales_channel(&id).await?;
    Ok(Json(channel))
}
