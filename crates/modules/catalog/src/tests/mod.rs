//! Catalog-module tests.

use fastcart_infrastructure::{StoreDatabase, fetch_pending_audits};
use fastcart_kernel::{AppError, Metadata, Pagination, StoreId};

use crate::{
    CatalogService, CreateCategoryInput, CreateCollectionInput, CreateImageInput,
    CreateOptionInput, CreateProductInput, CreateVariantInput, Dimensions, ProductFilter,
    ProductSort, ProductStatus, UpdateCategoryInput, UpdateCollectionInput, UpdateProductInput,
    UpdateVariantInput, VariantOptionInput,
};

async fn service() -> CatalogService {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    CatalogService::new(db)
}

async fn service_with_db() -> (CatalogService, StoreDatabase) {
    let db = StoreDatabase::in_memory(StoreId::new())
        .await
        .expect("in-memory store database");
    (CatalogService::new(db.clone()), db)
}

fn product_input(title: &str) -> CreateProductInput {
    CreateProductInput {
        title: title.to_owned(),
        ..Default::default()
    }
}

fn variant_input(title: &str, sku: &str) -> CreateVariantInput {
    CreateVariantInput {
        title: title.to_owned(),
        sku: Some(sku.to_owned()),
        ..Default::default()
    }
}

async fn basic_product(service: &CatalogService, title: &str, sku: &str) -> crate::Product {
    service
        .create_product(CreateProductInput {
            title: title.to_owned(),
            variants: vec![variant_input("Default", sku)],
            ..Default::default()
        })
        .await
        .expect("product")
}

async fn published_product(service: &CatalogService, title: &str, sku: &str) -> crate::Product {
    let product = basic_product(service, title, sku).await;
    service
        .publish_product(&product.id)
        .await
        .expect("published product")
}

async fn insert_sales_channel(db: &StoreDatabase, id: &str) {
    sqlx::query(
        "INSERT INTO sales_channels (id, name, is_disabled, metadata, created_at, updated_at) \
         VALUES (?, ?, 0, '{}', 0, 0)",
    )
    .bind(id)
    .bind(id)
    .execute(db.pool())
    .await
    .unwrap();
}

mod audit;
mod categories;
mod channels;
mod collections;
mod images;
mod products;
mod search;
mod tags;
mod validation;
mod variants;
