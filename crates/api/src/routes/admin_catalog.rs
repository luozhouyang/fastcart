//! Admin catalog, store-settings and sales-channel HTTP routes.
//!
//! Handlers translate HTTP requests into catalog and store service calls. All
//! business rules (validation, state transitions) live in the modules; this
//! layer only enforces permissions, shapes requests and maps responses.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use fastcart_http::extractors::ApiQuery;
use fastcart_http::{AdminActor, ApiError, ApiJson, ApiResult, PaginationQuery};
use fastcart_kernel::{
    ActorContext, AppError, CategoryId, CollectionId, ImageId, Metadata, Pagination, ProductId,
    TagId, VariantId,
};
use fastcart_module_catalog::{
    CatalogService, Category, Collection, CreateCategoryInput, CreateCollectionInput,
    CreateImageInput, CreateOptionInput, CreateProductInput, CreateVariantInput, Dimensions,
    Product, ProductFilter, ProductImage, ProductStatus, ProductVariant, Tag, UpdateCategoryInput,
    UpdateCollectionInput, UpdateProductInput, UpdateVariantInput, VariantOptionInput,
};
use fastcart_module_store::{
    CreateSalesChannelInput, SalesChannel, StoreService, UpdateSalesChannelInput,
};
use serde::Deserialize;

use super::StoreDb;
use crate::dto::PageResponse;
use crate::state::AppContext;

/// Builds the routes for this namespace.
pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/catalog/products", get(list_products).post(create_product))
        .route(
            "/catalog/products/{id}",
            get(get_product)
                .patch(update_product)
                .delete(delete_product),
        )
        .route("/catalog/products/{id}/publish", post(publish_product))
        .route("/catalog/products/{id}/archive", post(archive_product))
        .route("/catalog/products/{id}/variants", post(create_variant))
        .route(
            "/catalog/variants/{id}",
            get(get_variant)
                .patch(update_variant)
                .delete(delete_variant),
        )
        .route(
            "/catalog/products/{id}/images",
            get(list_images).post(add_image),
        )
        .route("/catalog/images/{id}", delete(remove_image))
        .route(
            "/catalog/products/{id}/images/reorder",
            post(reorder_images),
        )
        .route(
            "/catalog/categories",
            get(list_categories).post(create_category),
        )
        .route(
            "/catalog/categories/{id}",
            get(get_category)
                .patch(update_category)
                .delete(delete_category),
        )
        .route(
            "/catalog/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/catalog/collections/{id}",
            get(get_collection)
                .patch(update_collection)
                .delete(delete_collection),
        )
        .route(
            "/catalog/collections/{id}/publish",
            post(publish_collection),
        )
        .route(
            "/catalog/collections/{id}/unpublish",
            post(unpublish_collection),
        )
        .route(
            "/catalog/collections/{id}/products",
            post(add_collection_products),
        )
        .route(
            "/catalog/collections/{id}/products/{product_id}",
            delete(remove_collection_product),
        )
        .route("/catalog/tags", get(list_tags).post(create_tag))
        .route("/catalog/products/{id}/tags", post(attach_tag))
        .route("/catalog/products/{id}/tags/{tag_id}", delete(detach_tag))
        .route("/catalog/search", get(search_products))
        .route("/catalog/reindex", post(reindex))
        .route("/settings", get(get_settings).put(update_settings))
        .route(
            "/settings/{key}",
            get(get_setting).put(set_setting).delete(delete_setting),
        )
        .route(
            "/sales-channels",
            get(list_sales_channels).post(create_sales_channel),
        )
        .route(
            "/sales-channels/{id}",
            get(get_sales_channel).patch(update_sales_channel),
        )
        .route("/sales-channels/{id}/disable", post(disable_sales_channel))
        .route("/sales-channels/{id}/enable", post(enable_sales_channel))
}

// -------------------------------------------------------------------------
// Request shapes
// -------------------------------------------------------------------------

/// Query parameters for administrative product listing.
#[derive(Clone, Debug, Deserialize)]
struct ProductListQuery {
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

/// Query parameters for category listing.
#[derive(Clone, Debug, Deserialize)]
struct CategoryListQuery {
    #[serde(default)]
    parent_id: Option<String>,
}

/// Query parameters for administrative product search.
#[derive(Clone, Debug, Deserialize)]
struct SearchQuery {
    q: String,
}

/// A product option to create alongside a product.
#[derive(Clone, Debug, Deserialize)]
struct CreateOptionRequest {
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
#[derive(Clone, Debug, Deserialize)]
struct CreateVariantRequest {
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
#[derive(Clone, Debug, Deserialize)]
struct CreateProductRequest {
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

/// Category creation body.
#[derive(Clone, Debug, Deserialize)]
struct CreateCategoryRequest {
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

/// Collection creation body.
#[derive(Clone, Debug, Deserialize)]
struct CreateCollectionRequest {
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

/// Image creation body.
#[derive(Clone, Debug, Deserialize)]
struct CreateImageRequest {
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
#[derive(Clone, Debug, Deserialize)]
struct ImageReorderRequest {
    image_ids: Vec<String>,
}

/// Products to add to a collection.
#[derive(Clone, Debug, Deserialize)]
struct CollectionProductsRequest {
    product_ids: Vec<String>,
}

/// Tag creation body.
#[derive(Clone, Debug, Deserialize)]
struct TagRequest {
    value: String,
}

/// Body linking an existing tag to a product.
#[derive(Clone, Debug, Deserialize)]
struct ProductTagRequest {
    tag_id: String,
}

/// Batch settings update body.
#[derive(Clone, Debug, Deserialize)]
struct SettingsRequest {
    values: BTreeMap<String, String>,
}

/// Single setting update body.
#[derive(Clone, Debug, Deserialize)]
struct SettingRequest {
    value: String,
}

/// Sales-channel creation body.
#[derive(Clone, Debug, Deserialize)]
struct CreateSalesChannelRequest {
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

// -------------------------------------------------------------------------
// Helpers
// -------------------------------------------------------------------------

/// Parses an administrative product status filter strictly.
fn parse_product_status(value: &str) -> ApiResult<ProductStatus> {
    match value {
        "draft" => Ok(ProductStatus::Draft),
        "active" => Ok(ProductStatus::Active),
        "archived" => Ok(ProductStatus::Archived),
        _ => Err(ApiError::validation("invalid product status")),
    }
}

/// Enforces a permission at the handler boundary.
fn authorize(actor: &ActorContext, permission: &str) -> ApiResult<()> {
    actor.require_permission(permission).map_err(ApiError::new)
}

/// Resolves a tag identifier to its value.
///
/// The catalog service attaches tags by value only, so the identifier is
/// resolved by scanning the tag list.
async fn resolve_tag_value(service: &CatalogService, tag_id: &str) -> ApiResult<String> {
    let mut offset = 0_i64;
    loop {
        let page = service.list_tags(Pagination { limit: 100, offset }).await?;
        if let Some(tag) = page.data.iter().find(|tag| tag.id == tag_id) {
            return Ok(tag.value.clone());
        }
        offset += 100;
        if offset >= page.count {
            return Err(ApiError::new(AppError::not_found("tag", tag_id)));
        }
    }
}

// -------------------------------------------------------------------------
// Products
// -------------------------------------------------------------------------

async fn list_products(
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
        include_deleted: query.include_deleted,
    };
    let page = CatalogService::new(db)
        .list_products(filter, pagination)
        .await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_product(
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

async fn get_product(
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

async fn update_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateProductInput>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .update_product(product_id.as_str(), request)
        .await?;
    Ok(Json(product))
}

async fn delete_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    CatalogService::new(db)
        .delete_product(product_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn publish_product(
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

async fn archive_product(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<Product>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let product = CatalogService::new(db)
        .archive_product(product_id.as_str())
        .await?;
    Ok(Json(product))
}

// -------------------------------------------------------------------------
// Variants
// -------------------------------------------------------------------------

async fn create_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<CreateVariantRequest>,
) -> ApiResult<(StatusCode, Json<ProductVariant>)> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let variant = CatalogService::new(db)
        .create_variant(product_id.as_str(), CreateVariantInput::from(request))
        .await?;
    Ok((StatusCode::CREATED, Json(variant)))
}

async fn get_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<ProductVariant>> {
    authorize(&actor, "products.read")?;
    let variant_id = VariantId::from_string(id);
    let variant = CatalogService::new(db)
        .get_variant(variant_id.as_str())
        .await?;
    Ok(Json(variant))
}

async fn update_variant(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateVariantInput>,
) -> ApiResult<Json<ProductVariant>> {
    authorize(&actor, "products.write")?;
    let variant_id = VariantId::from_string(id);
    let variant = CatalogService::new(db)
        .update_variant(variant_id.as_str(), request)
        .await?;
    Ok(Json(variant))
}

async fn delete_variant(
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

// -------------------------------------------------------------------------
// Images
// -------------------------------------------------------------------------

async fn list_images(
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

async fn add_image(
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

async fn remove_image(
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

async fn reorder_images(
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

// -------------------------------------------------------------------------
// Categories
// -------------------------------------------------------------------------

async fn list_categories(
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

async fn create_category(
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

async fn get_category(
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

async fn update_category(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCategoryInput>,
) -> ApiResult<Json<Category>> {
    authorize(&actor, "products.write")?;
    let category_id = CategoryId::from_string(id);
    let category = CatalogService::new(db)
        .update_category(category_id.as_str(), request)
        .await?;
    Ok(Json(category))
}

async fn delete_category(
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

// -------------------------------------------------------------------------
// Collections
// -------------------------------------------------------------------------

async fn list_collections(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Collection>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db).list_collections(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_collection(
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

async fn get_collection(
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

async fn update_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateCollectionInput>,
) -> ApiResult<Json<Collection>> {
    authorize(&actor, "products.write")?;
    let collection_id = CollectionId::from_string(id);
    let collection = CatalogService::new(db)
        .update_collection(collection_id.as_str(), request)
        .await?;
    Ok(Json(collection))
}

async fn delete_collection(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    CatalogService::new(db).delete_collection(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn publish_collection(
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

async fn unpublish_collection(
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

async fn add_collection_products(
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

async fn remove_collection_product(
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

// -------------------------------------------------------------------------
// Tags
// -------------------------------------------------------------------------

async fn list_tags(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    PaginationQuery { limit, offset }: PaginationQuery,
) -> ApiResult<Json<PageResponse<Tag>>> {
    authorize(&actor, "products.read")?;
    let pagination = PaginationQuery { limit, offset }.pagination()?;
    let page = CatalogService::new(db).list_tags(pagination).await?;
    Ok(Json(PageResponse::from_page(page)))
}

async fn create_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<TagRequest>,
) -> ApiResult<(StatusCode, Json<Tag>)> {
    authorize(&actor, "products.write")?;
    let tag = CatalogService::new(db).create_tag(&request.value).await?;
    Ok((StatusCode::CREATED, Json(tag)))
}

async fn attach_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<ProductTagRequest>,
) -> ApiResult<Json<Tag>> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(id);
    let service = CatalogService::new(db);
    let value = resolve_tag_value(&service, &request.tag_id).await?;
    let tag = service.attach_tag(product_id.as_str(), &value).await?;
    Ok(Json(tag))
}

async fn detach_tag(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path((product_id, tag_id)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "products.write")?;
    let product_id = ProductId::from_string(product_id);
    let tag_id = TagId::from_string(tag_id);
    CatalogService::new(db)
        .detach_tag(product_id.as_str(), tag_id.as_str())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------
// Search
// -------------------------------------------------------------------------

async fn search_products(
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

async fn reindex(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&actor, "products.write")?;
    CatalogService::new(db).reindex_search().await?;
    Ok(Json(serde_json::json!({ "indexed": true })))
}

// -------------------------------------------------------------------------
// Settings
// -------------------------------------------------------------------------

async fn get_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<BTreeMap<String, String>>> {
    authorize(&actor, "settings.read")?;
    let settings = StoreService::new(db).get_all_settings().await?;
    Ok(Json(settings))
}

async fn update_settings(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    ApiJson(request): ApiJson<SettingsRequest>,
) -> ApiResult<Json<BTreeMap<String, String>>> {
    authorize(&actor, "settings.write")?;
    let service = StoreService::new(db);
    for (key, value) in &request.values {
        service.set_setting(key, value).await?;
    }
    let settings = service.get_all_settings().await?;
    Ok(Json(settings))
}

async fn get_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&actor, "settings.read")?;
    let value = StoreService::new(db)
        .get_setting(&key)
        .await?
        .ok_or_else(|| ApiError::new(AppError::not_found("setting", &key)))?;
    Ok(Json(serde_json::json!({ "key": key, "value": value })))
}

async fn set_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
    ApiJson(request): ApiJson<SettingRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    authorize(&actor, "settings.write")?;
    let service = StoreService::new(db);
    service.set_setting(&key, &request.value).await?;
    Ok(Json(
        serde_json::json!({ "key": key, "value": request.value }),
    ))
}

async fn delete_setting(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(key): Path<String>,
) -> ApiResult<StatusCode> {
    authorize(&actor, "settings.write")?;
    if !StoreService::new(db).delete_setting(&key).await? {
        return Err(ApiError::new(AppError::not_found("setting", &key)));
    }
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------
// Sales channels
// -------------------------------------------------------------------------

async fn list_sales_channels(
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

async fn create_sales_channel(
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

async fn get_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.read")?;
    let channel = StoreService::new(db).get_sales_channel(&id).await?;
    Ok(Json(channel))
}

async fn update_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
    ApiJson(request): ApiJson<UpdateSalesChannelInput>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db)
        .update_sales_channel(&id, request)
        .await?;
    Ok(Json(channel))
}

async fn disable_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db).disable_sales_channel(&id).await?;
    Ok(Json(channel))
}

async fn enable_sales_channel(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
    Path(id): Path<String>,
) -> ApiResult<Json<SalesChannel>> {
    authorize(&actor, "settings.write")?;
    let channel = StoreService::new(db).enable_sales_channel(&id).await?;
    Ok(Json(channel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_status_parsing_is_strict() {
        assert_eq!(parse_product_status("draft").unwrap(), ProductStatus::Draft);
        assert_eq!(
            parse_product_status("active").unwrap(),
            ProductStatus::Active
        );
        assert_eq!(
            parse_product_status("archived").unwrap(),
            ProductStatus::Archived
        );
        assert!(parse_product_status("published").is_err());
    }

    #[test]
    fn create_product_request_fills_defaults() {
        let request: CreateProductRequest =
            serde_json::from_str(r#"{"title": "Tee"}"#).expect("request");
        let input = CreateProductInput::from(request);
        assert_eq!(input.title, "Tee");
        assert!(input.variants.is_empty());
        assert!(input.options.is_empty());
        assert!(input.category_ids.is_empty());
        assert!(input.collection_ids.is_empty());
        assert!(input.tag_values.is_empty());
        assert!(input.images.is_empty());
        assert!(input.metadata.is_empty());
    }

    #[test]
    fn create_variant_request_maps_nested_defaults() {
        let request: CreateVariantRequest =
            serde_json::from_str(r#"{"title": "Small"}"#).expect("request");
        let input = CreateVariantInput::from(request);
        assert_eq!(input.title, "Small");
        assert!(input.option_value_ids.is_empty());
        assert!(input.option_values.is_empty());
        assert!(input.status.is_none());
    }

    #[test]
    fn create_collection_request_defaults_to_unpublished() {
        let request: CreateCollectionRequest =
            serde_json::from_str(r#"{"title": "Summer"}"#).expect("request");
        let input = CreateCollectionInput::from(request);
        assert_eq!(input.title, "Summer");
        assert!(input.is_published.is_none());
        assert!(input.metadata.is_empty());
    }
}
