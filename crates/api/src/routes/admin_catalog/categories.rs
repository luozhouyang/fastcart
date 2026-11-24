use super::*;

/// Query parameters for category listing.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct CategoryListQuery {
    #[serde(default)]
    parent_id: Option<String>,
}

/// Category creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateCategoryRequest {
    title: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    is_internal: Option<bool>,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateCategoryRequest> for CreateCategoryInput {
    fn from(request: CreateCategoryRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            parent_id: request.parent_id,
            description: request.description,
            is_active: request.is_active,
            is_internal: request.is_internal,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

/// Partial category update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateCategoryRequest {
    title: Option<String>,
    handle: Option<String>,
    parent_id: Option<String>,
    description: Option<String>,
    is_active: Option<bool>,
    is_internal: Option<bool>,
    rank: Option<i64>,
    metadata: Option<Metadata>,
}

impl From<UpdateCategoryRequest> for UpdateCategoryInput {
    fn from(request: UpdateCategoryRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            parent_id: request.parent_id,
            description: request.description,
            is_active: request.is_active,
            is_internal: request.is_internal,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

#[utoipa::path(
    get,
    path = "/catalog/categories",
    tag = "Admin",
    summary = "List categories",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, CategoryListQuery),
    responses((status = 200, body = PageResponse<Category>))
)]
pub(super) async fn list_categories(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<CategoryListQuery>,
) -> ApiResult<Json<PageResponse<Category>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .list_categories(query.parent_id.as_deref(), pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/catalog/categories",
    tag = "Admin",
    summary = "Create a category",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateCategoryRequest,
    responses((status = 201, body = Category))
)]
pub(super) async fn create_category(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateCategoryRequest>,
) -> ApiResult<(StatusCode, Json<Category>)> {
    authorize(&actor, "products.write")?;
    let category = CatalogService::new(db)
        .create_category(CreateCategoryInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(category)))
}

#[utoipa::path(
    get,
    path = "/catalog/categories/{id}",
    tag = "Admin",
    summary = "Get a category",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Category))
)]
pub(super) async fn get_category(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Category>> {
    authorize(&actor, "products.read")?;
    let category_id = CategoryId::from_string(id);
    let category = CatalogService::new(db)
        .get_category(category_id.as_str())
        .await?;
    Ok(Json(category))
}

#[utoipa::path(
    patch,
    path = "/catalog/categories/{id}",
    tag = "Admin",
    summary = "Update a category",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateCategoryRequest,
    responses((status = 200, body = Category))
)]
pub(super) async fn update_category(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCategoryRequest>,
) -> ApiResult<Json<Category>> {
    authorize(&actor, "products.write")?;
    let category_id = CategoryId::from_string(id);
    let category = CatalogService::new(db)
        .update_category(category_id.as_str(), request.into())
        .await?;
    Ok(Json(category))
}

#[utoipa::path(
    delete,
    path = "/catalog/categories/{id}",
    tag = "Admin",
    summary = "Delete a category",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_category(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let category_id = CategoryId::from_string(id);
    CatalogService::new(db)
        .delete_category(category_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
