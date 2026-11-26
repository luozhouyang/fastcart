use super::*;

/// Product type creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateProductTypeRequest {
    value: String,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateProductTypeRequest> for CreateProductTypeInput {
    fn from(request: CreateProductTypeRequest) -> Self {
        Self {
            value: request.value,
            metadata: request.metadata,
        }
    }
}

/// Partial product type update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateProductTypeRequest {
    value: Option<String>,
    metadata: Option<Metadata>,
}

impl From<UpdateProductTypeRequest> for UpdateProductTypeInput {
    fn from(request: UpdateProductTypeRequest) -> Self {
        Self {
            value: request.value,
            metadata: request.metadata,
        }
    }
}

#[utoipa::path(
    get,
    path = "/catalog/product-types",
    tag = "Admin",
    summary = "List product types",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<ProductType>))
)]
pub(super) async fn list_product_types(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<ProductType>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_product_types(pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/catalog/product-types",
    tag = "Admin",
    summary = "Create a product type",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateProductTypeRequest,
    responses((status = 201, body = ProductType))
)]
pub(super) async fn create_product_type(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateProductTypeRequest>,
) -> ApiResult<(StatusCode, Json<ProductType>)> {
    authorize(&actor, "products.write")?;
    let product_type = CatalogService::new(db)
        .create_product_type(CreateProductTypeInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(product_type)))
}

#[utoipa::path(
    get,
    path = "/catalog/product-types/{id}",
    tag = "Admin",
    summary = "Get a product type",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ProductType))
)]
pub(super) async fn get_product_type(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ProductType>> {
    authorize(&actor, "products.read")?;
    let product_type_id = ProductTypeId::from_string(id);
    let product_type = CatalogService::new(db)
        .get_product_type(product_type_id.as_str())
        .await?;
    Ok(Json(product_type))
}

#[utoipa::path(
    patch,
    path = "/catalog/product-types/{id}",
    tag = "Admin",
    summary = "Update a product type",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateProductTypeRequest,
    responses((status = 200, body = ProductType))
)]
pub(super) async fn update_product_type(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateProductTypeRequest>,
) -> ApiResult<Json<ProductType>> {
    authorize(&actor, "products.write")?;
    let product_type_id = ProductTypeId::from_string(id);
    let product_type = CatalogService::new(db)
        .update_product_type(product_type_id.as_str(), request.into())
        .await?;
    Ok(Json(product_type))
}

#[utoipa::path(
    delete,
    path = "/catalog/product-types/{id}",
    tag = "Admin",
    summary = "Delete a product type",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_product_type(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let product_type_id = ProductTypeId::from_string(id);
    CatalogService::new(db)
        .delete_product_type(product_type_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
