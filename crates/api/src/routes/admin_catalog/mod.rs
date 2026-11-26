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
    ProductTypeId, TagId, VariantId,
};
use fastcart_module_catalog::{
    CatalogService, Category, Collection, CreateCategoryInput, CreateCollectionInput,
    CreateImageInput, CreateOptionInput, CreateProductInput, CreateProductTypeInput,
    CreateVariantInput, Dimensions, Product, ProductFilter, ProductImage, ProductStatus,
    ProductType, Tag, UpdateCategoryInput, UpdateCollectionInput, UpdateProductInput,
    UpdateProductTypeInput, UpdateVariantInput, VariantOptionInput,
};
use fastcart_module_store::{
    CreateSalesChannelInput, SalesChannel, StoreService, UpdateSalesChannelInput,
};
use serde::{Deserialize, Serialize};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::StoreDb;
use crate::dto::{PageResponse, VariantDto};
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
            "/products/{id}/sales-channels",
            post(set_product_sales_channels),
        )
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
            "/catalog/product-types",
            get(list_product_types).post(create_product_type),
        )
        .route(
            "/catalog/product-types/{id}",
            get(get_product_type)
                .patch(update_product_type)
                .delete(delete_product_type),
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

/// Builds the documented admin catalog routes for OpenAPI generation.
pub fn openapi_router() -> OpenApiRouter<Arc<AppContext>> {
    OpenApiRouter::new()
        .routes(routes!(list_products, create_product))
        .routes(routes!(get_product, update_product, delete_product))
        .routes(routes!(publish_product))
        .routes(routes!(archive_product))
        .routes(routes!(create_variant))
        .routes(routes!(set_product_sales_channels))
        .routes(routes!(get_variant, update_variant, delete_variant))
        .routes(routes!(list_images, add_image))
        .routes(routes!(remove_image))
        .routes(routes!(reorder_images))
        .routes(routes!(list_categories, create_category))
        .routes(routes!(get_category, update_category, delete_category))
        .routes(routes!(list_product_types, create_product_type))
        .routes(routes!(
            get_product_type,
            update_product_type,
            delete_product_type
        ))
        .routes(routes!(list_collections, create_collection))
        .routes(routes!(
            get_collection,
            update_collection,
            delete_collection
        ))
        .routes(routes!(publish_collection))
        .routes(routes!(unpublish_collection))
        .routes(routes!(add_collection_products))
        .routes(routes!(remove_collection_product))
        .routes(routes!(list_tags, create_tag))
        .routes(routes!(attach_tag))
        .routes(routes!(detach_tag))
        .routes(routes!(search_products))
        .routes(routes!(reindex))
        .routes(routes!(get_settings, update_settings))
        .routes(routes!(get_setting, set_setting, delete_setting))
        .routes(routes!(list_sales_channels, create_sales_channel))
        .routes(routes!(get_sales_channel, update_sales_channel))
        .routes(routes!(disable_sales_channel))
        .routes(routes!(enable_sales_channel))
}

// -------------------------------------------------------------------------
// Request shapes
// -------------------------------------------------------------------------

mod categories;
mod collections;
mod product_types;
mod products;
mod sales_channels;
mod settings;
mod tags;

use categories::*;
use collections::*;
use product_types::*;
use products::*;
use sales_channels::*;
use settings::*;
use tags::*;

pub use categories::{CreateCategoryRequest, UpdateCategoryRequest};
pub use collections::{
    CollectionProductsRequest, CreateCollectionRequest, UpdateCollectionRequest,
};
pub use product_types::{CreateProductTypeRequest, UpdateProductTypeRequest};
pub use products::{
    CreateImageRequest, CreateOptionRequest, CreateProductRequest, CreateVariantRequest,
    ImageReorderRequest, ProductSalesChannelsRequest, UpdateProductRequest, UpdateVariantRequest,
};
pub use sales_channels::{CreateSalesChannelRequest, UpdateSalesChannelRequest};
pub use settings::{SettingRequest, SettingsRequest};
pub use tags::{ProductTagRequest, TagRequest};

/// Result of rebuilding the catalog search index.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct ReindexResponse {
    indexed: bool,
}

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

#[utoipa::path(
    post,
    path = "/catalog/reindex",
    tag = "Admin",
    summary = "Reindex the product search catalog",
    security(("adminSession" = []), ("apiToken" = [])),
    responses((status = 200, body = ReindexResponse))
)]
async fn reindex(
    AdminActor(actor): AdminActor,
    StoreDb(db): StoreDb,
) -> ApiResult<Json<ReindexResponse>> {
    authorize(&actor, "products.write")?;
    CatalogService::new(db).reindex_search().await?;
    Ok(Json(ReindexResponse { indexed: true }))
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
