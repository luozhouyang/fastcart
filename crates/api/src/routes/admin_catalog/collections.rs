use super::*;

/// Collection creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCollectionRequest {
    title: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_published: Option<bool>,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateCollectionRequest> for CreateCollectionInput {
    fn from(request: CreateCollectionRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            description: request.description,
            is_published: request.is_published,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

/// Products to add to a collection.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CollectionProductsRequest {
    product_ids: Vec<String>,
}

/// Partial collection update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateCollectionRequest {
    title: Option<String>,
    handle: Option<String>,
    description: Option<String>,
    is_published: Option<bool>,
    rank: Option<i64>,
    metadata: Option<Metadata>,
}

impl From<UpdateCollectionRequest> for UpdateCollectionInput {
    fn from(request: UpdateCollectionRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            description: request.description,
            is_published: request.is_published,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

#[utoipa::path(
    get,
    path = "/catalog/collections",
    tag = "Admin",
    summary = "List collections",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery),
    responses((status = 200, body = PageResponse<Collection>))
)]
pub(super) async fn list_collections(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Collection>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db).list_collections(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/catalog/collections",
    tag = "Admin",
    summary = "Create a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateCollectionRequest,
    responses((status = 201, body = Collection))
)]
pub(super) async fn create_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCollectionRequest>,
) -> ApiResult<(StatusCode, Json<Collection>)> {
    authorize(&actor, "products.write")?;
    let collection = CatalogService::new(db)
        .create_collection(CreateCollectionInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

#[utoipa::path(
    get,
    path = "/catalog/collections/{id}",
    tag = "Admin",
    summary = "Get a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Collection))
)]
pub(super) async fn get_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    authorize(&actor, "products.read")?;
    let collection_id = CollectionId::from_string(id);
    let collection = CatalogService::new(db)
        .get_collection(collection_id.as_str())
        .await?;
    Ok(Json(collection))
}

#[utoipa::path(
    patch,
    path = "/catalog/collections/{id}",
    tag = "Admin",
    summary = "Update a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateCollectionRequest,
    responses((status = 200, body = Collection))
)]
pub(super) async fn update_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCollectionRequest>,
) -> ApiResult<Json<Collection>> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(id);
    let collection = CatalogService::new(db)
        .update_collection(collection_id.as_str(), request.into())
        .await?;
    Ok(Json(collection))
}

#[utoipa::path(
    delete,
    path = "/catalog/collections/{id}",
    tag = "Admin",
    summary = "Delete a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    CatalogService::new(db).delete_collection(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/catalog/collections/{id}/publish",
    tag = "Admin",
    summary = "Publish a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Collection))
)]
pub(super) async fn publish_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(id);
    let collection = CatalogService::new(db)
        .publish_collection(collection_id.as_str())
        .await?;
    Ok(Json(collection))
}

#[utoipa::path(
    post,
    path = "/catalog/collections/{id}/unpublish",
    tag = "Admin",
    summary = "Unpublish a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Collection))
)]
pub(super) async fn unpublish_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Collection>> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(id);
    let collection = CatalogService::new(db)
        .unpublish_collection(collection_id.as_str())
        .await?;
    Ok(Json(collection))
}

#[utoipa::path(
    post,
    path = "/catalog/collections/{id}/products",
    tag = "Admin",
    summary = "Add products to a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CollectionProductsRequest,
    responses((status = 204))
)]
pub(super) async fn add_collection_products(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CollectionProductsRequest>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(id);
    let service = CatalogService::new(db);
    for (rank, product_id) in request.product_ids.iter().enumerate() {
        let rank = i64::try_from(rank).map_err(|_| ApiError::validation("too many products"))?;
        service
            .add_product_to_collection(product_id, collection_id.as_str(), rank)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/catalog/collections/{id}/products/{product_id}",
    tag = "Admin",
    summary = "Remove a product from a collection",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn remove_collection_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((collection_id, product_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(collection_id);
    let product_id = ProductId::from_string(product_id);
    CatalogService::new(db)
        .remove_product_from_collection(product_id.as_str(), collection_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
