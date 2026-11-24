use super::*;

/// Query parameters for administrative product listing.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct ProductListQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    collection_id: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    include_deleted: bool,
}

/// Query parameters for administrative product search.
#[derive(Clone, Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(super) struct SearchQuery {
    q: String,
}

/// A product option to create alongside a product.
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct CreateOptionRequest {
    title: String,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateOptionRequest> for CreateOptionInput {
    fn from(request: CreateOptionRequest) -> Self {
        Self {
            title: request.title,
            values: request.values,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

/// A variant to create alongside a product.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateVariantRequest {
    title: String,
    #[serde(default)]
    sku: Option<String>,
    #[serde(default)]
    barcode: Option<String>,
    #[serde(default)]
    status: Option<ProductStatus>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    dimensions: Option<Dimensions>,
    #[serde(default)]
    manage_inventory: Option<bool>,
    #[serde(default)]
    allow_backorder: Option<bool>,
    #[serde(default)]
    tax_category_id: Option<String>,
    #[serde(default)]
    metadata: Metadata,
    #[serde(default)]
    option_value_ids: Vec<String>,
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    option_values: Vec<VariantOptionInput>,
}

impl From<CreateVariantRequest> for CreateVariantInput {
    fn from(request: CreateVariantRequest) -> Self {
        Self {
            title: request.title,
            sku: request.sku,
            barcode: request.barcode,
            status: request.status,
            weight: request.weight,
            dimensions: request.dimensions,
            manage_inventory: request.manage_inventory,
            allow_backorder: request.allow_backorder,
            tax_category_id: request.tax_category_id,
            metadata: request.metadata,
            option_value_ids: request.option_value_ids,
            option_values: request.option_values,
        }
    }
}

/// Product creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateProductRequest {
    title: String,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    dimensions: Option<Dimensions>,
    #[serde(default)]
    metadata: Metadata,
    #[serde(default)]
    variants: Vec<CreateVariantRequest>,
    #[serde(default)]
    options: Vec<CreateOptionRequest>,
    #[serde(default)]
    category_ids: Vec<String>,
    #[serde(default)]
    collection_ids: Vec<String>,
    #[serde(default)]
    tag_values: Vec<String>,
    #[serde(default)]
    images: Vec<String>,
}

impl From<CreateProductRequest> for CreateProductInput {
    fn from(request: CreateProductRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            subtitle: request.subtitle,
            description: request.description,
            thumbnail: request.thumbnail,
            weight: request.weight,
            dimensions: request.dimensions,
            metadata: request.metadata,
            variants: request.variants.into_iter().map(Into::into).collect(),
            options: request.options.into_iter().map(Into::into).collect(),
            category_ids: request.category_ids,
            collection_ids: request.collection_ids,
            tag_values: request.tag_values,
            images: request.images,
        }
    }
}

/// Partial product update body.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateProductRequest {
    title: Option<String>,
    handle: Option<String>,
    subtitle: Option<String>,
    description: Option<String>,
    thumbnail: Option<String>,
    status: Option<ProductStatus>,
    weight: Option<f64>,
    dimensions: Option<Dimensions>,
    metadata: Option<Metadata>,
    variants: Option<Vec<UpdateVariantRequest>>,
    options: Option<Vec<CreateOptionRequest>>,
    category_ids: Option<Vec<String>>,
    collection_ids: Option<Vec<String>>,
    tag_values: Option<Vec<String>>,
    images: Option<Vec<String>>,
}

impl From<UpdateProductRequest> for UpdateProductInput {
    fn from(request: UpdateProductRequest) -> Self {
        Self {
            title: request.title,
            handle: request.handle,
            subtitle: request.subtitle,
            description: request.description,
            thumbnail: request.thumbnail,
            status: request.status,
            weight: request.weight,
            dimensions: request.dimensions,
            metadata: request.metadata,
            variants: request
                .variants
                .map(|variants| variants.into_iter().map(Into::into).collect()),
            options: request
                .options
                .map(|options| options.into_iter().map(Into::into).collect()),
            category_ids: request.category_ids,
            collection_ids: request.collection_ids,
            tag_values: request.tag_values,
            images: request.images,
        }
    }
}

/// Partial variant update body.
#[derive(Clone, Debug, Default, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(default)]
pub struct UpdateVariantRequest {
    id: Option<String>,
    title: Option<String>,
    sku: Option<String>,
    barcode: Option<String>,
    status: Option<ProductStatus>,
    weight: Option<f64>,
    dimensions: Option<Dimensions>,
    manage_inventory: Option<bool>,
    allow_backorder: Option<bool>,
    tax_category_id: Option<String>,
    metadata: Option<Metadata>,
    option_value_ids: Option<Vec<String>>,
}

impl From<UpdateVariantRequest> for UpdateVariantInput {
    fn from(request: UpdateVariantRequest) -> Self {
        Self {
            id: request.id,
            title: request.title,
            sku: request.sku,
            barcode: request.barcode,
            status: request.status,
            weight: request.weight,
            dimensions: request.dimensions,
            manage_inventory: request.manage_inventory,
            allow_backorder: request.allow_backorder,
            tax_category_id: request.tax_category_id,
            metadata: request.metadata,
            option_value_ids: request.option_value_ids,
        }
    }
}

/// Image creation body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateImageRequest {
    url: String,
    #[serde(default)]
    alt: Option<String>,
    #[serde(default)]
    variant_id: Option<String>,
    #[serde(default)]
    rank: Option<i64>,
    #[serde(default)]
    metadata: Metadata,
}

impl From<CreateImageRequest> for CreateImageInput {
    fn from(request: CreateImageRequest) -> Self {
        Self {
            url: request.url,
            alt: request.alt,
            variant_id: request.variant_id,
            rank: request.rank,
            metadata: request.metadata,
        }
    }
}

/// Image reorder body.
#[derive(Clone, Debug, Deserialize, utoipa::ToSchema)]
pub struct ImageReorderRequest {
    image_ids: Vec<String>,
}

/// Body replacing the sales channels a product is published to.
#[derive(Clone, Debug, Default, Deserialize, utoipa::ToSchema)]
#[schema(as = SetProductSalesChannelsRequest)]
pub struct ProductSalesChannelsRequest {
    #[serde(default)]
    sales_channel_ids: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/catalog/products",
    tag = "Admin",
    summary = "List products",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, ProductListQuery),
    responses((status = 200, body = PageResponse<Product>))
)]
pub(super) async fn list_products(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<ProductListQuery>,
) -> ApiResult<Json<PageResponse<Product>>> {
    authorize(&actor, "products.read")?;
    let status = query
        .status
        .as_deref()
        .map(parse_product_status)
        .transpose()?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let filter = ProductFilter {
        status,
        category_id: query.category_id,
        collection_id: query.collection_id,
        tag_value: query.tag,
        query: query.query,
        sales_channel_ids: None,
        include_deleted: query.include_deleted,
    };
    let page = CatalogService::new(db)
        .list_products(filter, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

#[utoipa::path(
    post,
    path = "/catalog/products",
    tag = "Admin",
    summary = "Create a product",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateProductRequest,
    responses((status = 201, body = Product))
)]
pub(super) async fn create_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<CreateProductRequest>,
) -> ApiResult<(StatusCode, Json<Product>)> {
    authorize(&actor, "products.write")?;
    let product = CatalogService::new(db)
        .create_product(CreateProductInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(product)))
}

#[utoipa::path(
    get,
    path = "/catalog/products/{id}",
    tag = "Admin",
    summary = "Get a product",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Product))
)]
pub(super) async fn get_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.read")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .get_product(product_id.as_str())
        .await?;
    Ok(Json(product))
}

#[utoipa::path(
    patch,
    path = "/catalog/products/{id}",
    tag = "Admin",
    summary = "Update a product",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateProductRequest,
    responses((status = 200, body = Product))
)]
pub(super) async fn update_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateProductRequest>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .update_product(product_id.as_str(), request.into())
        .await?;
    Ok(Json(product))
}

#[utoipa::path(
    delete,
    path = "/catalog/products/{id}",
    tag = "Admin",
    summary = "Delete a product",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    CatalogService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .delete_product(product_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/publish",
    tag = "Admin",
    summary = "Publish a product",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Product))
)]
pub(super) async fn publish_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .publish_product(product_id.as_str())
        .await?;
    Ok(Json(product))
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/archive",
    tag = "Admin",
    summary = "Archive a product",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Product))
)]
pub(super) async fn archive_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .with_audit_actor(actor.user_id.as_ref().map(ToString::to_string))
        .archive_product(product_id.as_str())
        .await?;
    Ok(Json(product))
}

/// Replaces the sales channels a product is published to.
#[utoipa::path(
    post,
    path = "/products/{id}/sales-channels",
    tag = "Admin",
    summary = "Replace the sales channels a product is published to",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ProductSalesChannelsRequest,
    responses((status = 200, body = Product))
)]
pub(super) async fn set_product_sales_channels(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<ProductSalesChannelsRequest>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let service =
        CatalogService::new(db).with_audit_actor(actor.user_id.as_ref().map(ToString::to_string));
    service
        .set_product_sales_channels(product_id.as_str(), &request.sales_channel_ids)
        .await?;
    let product = service.get_product(product_id.as_str()).await?;
    Ok(Json(product))
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/variants",
    tag = "Admin",
    summary = "Create a product variant",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateVariantRequest,
    responses((status = 201, body = VariantDto))
)]
pub(super) async fn create_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateVariantRequest>,
) -> ApiResult<(StatusCode, Json<VariantDto>)> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let variant = CatalogService::new(db)
        .create_variant(product_id.as_str(), CreateVariantInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(variant.into())))
}

#[utoipa::path(
    get,
    path = "/catalog/variants/{id}",
    tag = "Admin",
    summary = "Get a variant",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = VariantDto))
)]
pub(super) async fn get_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<VariantDto>> {
    authorize(&actor, "products.read")?;
    let variant_id = VariantId::from_string(id);
    let variant = CatalogService::new(db)
        .get_variant(variant_id.as_str())
        .await?;
    Ok(Json(variant.into()))
}

#[utoipa::path(
    patch,
    path = "/catalog/variants/{id}",
    tag = "Admin",
    summary = "Update a variant",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = UpdateVariantRequest,
    responses((status = 200, body = VariantDto))
)]
pub(super) async fn update_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateVariantRequest>,
) -> ApiResult<Json<VariantDto>> {
    authorize(&actor, "products.write")?;
    let variant_id = VariantId::from_string(id);
    let variant = CatalogService::new(db)
        .update_variant(variant_id.as_str(), request.into())
        .await?;
    Ok(Json(variant.into()))
}

#[utoipa::path(
    delete,
    path = "/catalog/variants/{id}",
    tag = "Admin",
    summary = "Delete a variant",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn delete_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let variant_id = VariantId::from_string(id);
    CatalogService::new(db)
        .delete_variant(variant_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/catalog/products/{id}/images",
    tag = "Admin",
    summary = "List product images",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = Vec<ProductImage>))
)]
pub(super) async fn list_images(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<ProductImage>>> {
    authorize(&actor, "products.read")?;
    let product_id = ProductId::from_string(id);
    let images = CatalogService::new(db)
        .list_images(product_id.as_str())
        .await?;
    Ok(Json(images))
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/images",
    tag = "Admin",
    summary = "Add a product image",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = CreateImageRequest,
    responses((status = 201, body = ProductImage))
)]
pub(super) async fn add_image(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateImageRequest>,
) -> ApiResult<(StatusCode, Json<ProductImage>)> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let image = CatalogService::new(db)
        .add_image(product_id.as_str(), CreateImageInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(image)))
}

#[utoipa::path(
    delete,
    path = "/catalog/images/{id}",
    tag = "Admin",
    summary = "Remove a product image",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 204))
)]
pub(super) async fn remove_image(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let image_id = ImageId::from_string(id);
    CatalogService::new(db)
        .remove_image(image_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/catalog/products/{id}/images/reorder",
    tag = "Admin",
    summary = "Reorder product images",
    security(("adminSession" = []), ("apiToken" = [])),
    request_body = ImageReorderRequest,
    responses((status = 200, body = Vec<ProductImage>))
)]
pub(super) async fn reorder_images(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<ImageReorderRequest>,
) -> ApiResult<Json<Vec<ProductImage>>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let images = CatalogService::new(db)
        .reorder_images(product_id.as_str(), &request.image_ids)
        .await?;
    Ok(Json(images))
}

#[utoipa::path(
    get,
    path = "/catalog/search",
    tag = "Admin",
    summary = "Search products as an administrator",
    security(("adminSession" = []), ("apiToken" = [])),
    params(PaginationQuery, SearchQuery),
    responses((status = 200, body = PageResponse<Product>))
)]
pub(super) async fn search_products(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<PageResponse<Product>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db)
        .search_products_admin(&query.q, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}
