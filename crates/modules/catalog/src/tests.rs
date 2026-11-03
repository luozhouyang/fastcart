//! Catalog-module tests.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, Pagination, StoreId};

use crate::{
    CatalogService, CreateCategoryInput, CreateCollectionInput, CreateImageInput,
    CreateOptionInput, CreateProductInput, CreateVariantInput, Dimensions, ProductFilter,
    ProductStatus, UpdateCategoryInput, UpdateCollectionInput, UpdateProductInput,
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

#[tokio::test]
async fn creates_and_reads_a_product_aggregate() {
    let service = service().await;
    let mut metadata = Metadata::empty();
    metadata.insert("material", serde_json::json!("cotton"));
    let product = service
        .create_product(CreateProductInput {
            title: "Classic T-Shirt".to_owned(),
            subtitle: Some("Soft cotton".to_owned()),
            description: Some("A everyday tee".to_owned()),
            thumbnail: Some("https://example.com/tee.jpg".to_owned()),
            weight: Some(200.0),
            dimensions: Some(Dimensions {
                length: Some(30.0),
                height: Some(2.0),
                width: Some(20.0),
            }),
            metadata,
            variants: vec![variant_input("Small", "TEE-S")],
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(product.id.starts_with("prod_"));
    assert_eq!(product.handle, "classic-t-shirt");
    assert_eq!(product.status, ProductStatus::Draft);
    assert_eq!(product.subtitle.as_deref(), Some("Soft cotton"));
    assert_eq!(product.weight, Some(200.0));
    assert_eq!(product.dimensions.width, Some(20.0));
    assert_eq!(
        product.metadata.get("material"),
        Some(&serde_json::json!("cotton"))
    );
    assert_eq!(product.variants.len(), 1);
    assert_eq!(product.variants[0].sku.as_deref(), Some("TEE-S"));
    assert!(product.variants[0].manage_inventory);
    assert!(!product.variants[0].allow_backorder);
    assert!(product.created_at > 0);

    let loaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(loaded, product);
    let by_handle = service
        .get_product_by_handle("classic-t-shirt")
        .await
        .unwrap();
    assert_eq!(by_handle.id, product.id);
}

#[tokio::test]
async fn product_handles_are_slugified_and_unique() {
    let service = service().await;
    let first = service
        .create_product(product_input("Summer Sale 2026!"))
        .await
        .unwrap();
    assert_eq!(first.handle, "summer-sale-2026");

    let error = service
        .create_product(CreateProductInput {
            handle: Some("summer-sale-2026".to_owned()),
            ..product_input("Other")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn invalid_product_handles_are_rejected() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            handle: Some("Not A Handle".to_owned()),
            ..product_input("Product")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn product_title_is_required_and_bounded() {
    let service = service().await;
    assert!(matches!(
        service
            .create_product(product_input("   "))
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
    assert!(matches!(
        service
            .create_product(product_input(&"x".repeat(201)))
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
}

#[tokio::test]
async fn duplicate_variant_skus_conflict_across_products() {
    let service = service().await;
    basic_product(&service, "First", "SKU-1").await;
    let error = service
        .create_product(CreateProductInput {
            variants: vec![variant_input("Default", "SKU-1")],
            ..product_input("Second")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn duplicate_skus_inside_one_product_conflict() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            variants: vec![variant_input("One", "SAME"), variant_input("Two", "SAME")],
            ..product_input("Duplicates")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn creates_multi_variant_products_with_options() {
    let service = service().await;
    let product = service
        .create_product(CreateProductInput {
            title: "T-Shirt".to_owned(),
            options: vec![CreateOptionInput {
                title: "Size".to_owned(),
                values: vec!["S".to_owned(), "M".to_owned()],
                ..Default::default()
            }],
            variants: vec![
                CreateVariantInput {
                    title: "Small".to_owned(),
                    sku: Some("TS-S".to_owned()),
                    option_values: vec![VariantOptionInput {
                        option_title: "size".to_owned(),
                        value: "s".to_owned(),
                    }],
                    ..Default::default()
                },
                CreateVariantInput {
                    title: "Medium".to_owned(),
                    sku: Some("TS-M".to_owned()),
                    option_values: vec![VariantOptionInput {
                        option_title: "Size".to_owned(),
                        value: "M".to_owned(),
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(product.options.len(), 1);
    assert_eq!(product.options[0].title, "Size");
    assert_eq!(product.options[0].values.len(), 2);
    assert_eq!(product.variants.len(), 2);
    for variant in &product.variants {
        assert_eq!(variant.option_value_ids.len(), 1);
        assert!(
            product.options[0]
                .values
                .iter()
                .any(|value| value.id == variant.option_value_ids[0])
        );
    }
}

#[tokio::test]
async fn unknown_variant_option_value_is_rejected() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            options: vec![CreateOptionInput {
                title: "Size".to_owned(),
                values: vec!["S".to_owned()],
                ..Default::default()
            }],
            variants: vec![CreateVariantInput {
                title: "Small".to_owned(),
                option_values: vec![VariantOptionInput {
                    option_title: "Size".to_owned(),
                    value: "XL".to_owned(),
                }],
                ..Default::default()
            }],
            ..product_input("Shirt")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn duplicate_option_values_are_rejected() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            options: vec![CreateOptionInput {
                title: "Colour".to_owned(),
                values: vec!["Red".to_owned(), "red".to_owned()],
                ..Default::default()
            }],
            ..product_input("Shirt")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn updates_product_fields_partially() {
    let service = service().await;
    let product = basic_product(&service, "Old Title", "SKU-U").await;
    let mut metadata = Metadata::empty();
    metadata.insert("season", serde_json::json!("winter"));

    let updated = service
        .update_product(
            &product.id,
            UpdateProductInput {
                title: Some("New Title".to_owned()),
                subtitle: Some("Subtitle".to_owned()),
                metadata: Some(metadata),
                weight: Some(12.5),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.handle, product.handle);
    assert_eq!(updated.subtitle.as_deref(), Some("Subtitle"));
    assert_eq!(updated.weight, Some(12.5));
    assert_eq!(
        updated.metadata.get("season"),
        Some(&serde_json::json!("winter"))
    );
    assert_eq!(updated.variants.len(), 1);
}

#[tokio::test]
async fn substituting_variants_replaces_the_set() {
    let service = service().await;
    let product = service
        .create_product(CreateProductInput {
            title: "Product".to_owned(),
            variants: vec![variant_input("One", "A"), variant_input("Two", "B")],
            ..Default::default()
        })
        .await
        .unwrap();
    let kept_id = product
        .variants
        .iter()
        .find(|variant| variant.sku.as_deref() == Some("A"))
        .unwrap()
        .id
        .clone();

    let updated = service
        .update_product(
            &product.id,
            UpdateProductInput {
                variants: Some(vec![
                    UpdateVariantInput {
                        id: Some(kept_id.clone()),
                        title: Some("One Updated".to_owned()),
                        ..Default::default()
                    },
                    UpdateVariantInput {
                        title: Some("Three".to_owned()),
                        sku: Some("C".to_owned()),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.variants.len(), 2);
    assert!(
        updated
            .variants
            .iter()
            .any(|variant| variant.id == kept_id && variant.title == "One Updated")
    );
    assert!(
        updated
            .variants
            .iter()
            .any(|variant| variant.sku.as_deref() == Some("C"))
    );
}

#[tokio::test]
async fn replaces_category_collection_and_tag_links() {
    let service = service().await;
    let category = service
        .create_category(CreateCategoryInput {
            title: "Apparel".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let collection = service
        .create_collection(CreateCollectionInput {
            title: "Summer".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let product = service
        .create_product(CreateProductInput {
            category_ids: vec![category.id.clone()],
            collection_ids: vec![collection.id.clone()],
            tag_values: vec!["new".to_owned()],
            ..product_input("Shirt")
        })
        .await
        .unwrap();

    assert_eq!(product.categories.len(), 1);
    assert_eq!(product.collections.len(), 1);
    assert_eq!(product.tags.len(), 1);
    assert_eq!(product.tags[0].value, "new");

    let updated = service
        .update_product(
            &product.id,
            UpdateProductInput {
                category_ids: Some(Vec::new()),
                collection_ids: Some(Vec::new()),
                tag_values: Some(vec!["sale".to_owned()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.categories.is_empty());
    assert!(updated.collections.is_empty());
    assert_eq!(updated.tags.len(), 1);
    assert_eq!(updated.tags[0].value, "sale");
}

#[tokio::test]
async fn deleting_a_product_is_a_soft_delete() {
    let service = service().await;
    let product = basic_product(&service, "Delete Me", "DEL-1").await;
    service.delete_product(&product.id).await.unwrap();

    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .get_product_by_handle(&product.handle)
            .await
            .unwrap_err()
            .is_not_found()
    );
    let admin = service
        .list_products(
            ProductFilter {
                include_deleted: true,
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(admin.count, 1);
    let visible = service
        .list_products(ProductFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(visible.count, 0);
    assert!(
        service
            .delete_product(&product.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn publish_and_archive_follow_lifecycle_rules() {
    let service = service().await;
    let product = basic_product(&service, "Lifecycle", "LIFE-1").await;

    let published = service.publish_product(&product.id).await.unwrap();
    assert_eq!(published.status, ProductStatus::Active);
    assert_eq!(
        service.publish_product(&product.id).await.unwrap().status,
        ProductStatus::Active
    );

    let archived = service.archive_product(&product.id).await.unwrap();
    assert_eq!(archived.status, ProductStatus::Archived);
    assert!(matches!(
        service.publish_product(&product.id).await.unwrap_err(),
        AppError::Conflict(_)
    ));
    assert!(matches!(
        service
            .update_product(
                &product.id,
                UpdateProductInput {
                    status: Some(ProductStatus::Active),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err(),
        AppError::Conflict(_)
    ));
}

#[tokio::test]
async fn storefront_reads_only_expose_active_products() {
    let service = service().await;
    let product = basic_product(&service, "Storefront", "SF-1").await;

    assert_eq!(
        service
            .list_published(Pagination::default())
            .await
            .unwrap()
            .count,
        0
    );
    assert!(
        service
            .get_published_by_handle(&product.handle)
            .await
            .unwrap_err()
            .is_not_found()
    );

    service.publish_product(&product.id).await.unwrap();
    let published = service.list_published(Pagination::default()).await.unwrap();
    assert_eq!(published.count, 1);
    assert_eq!(published.data[0].variants.len(), 1);
    assert_eq!(
        service
            .get_published_by_handle(&product.handle)
            .await
            .unwrap()
            .id,
        product.id
    );

    service.archive_product(&product.id).await.unwrap();
    assert_eq!(
        service
            .list_published(Pagination::default())
            .await
            .unwrap()
            .count,
        0
    );
    assert!(
        service
            .get_published_by_handle(&product.handle)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn lists_products_with_filters_and_pagination() {
    let service = service().await;
    let category = service
        .create_category(CreateCategoryInput {
            title: "Shoes".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let collection = service
        .create_collection(CreateCollectionInput {
            title: "Featured".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let mut first = CreateProductInput {
        category_ids: vec![category.id.clone()],
        collection_ids: vec![collection.id.clone()],
        tag_values: vec!["featured".to_owned()],
        variants: vec![variant_input("Default", "F-1")],
        ..product_input("Running Shoe")
    };
    first.description = Some("Lightweight trainer".to_owned());
    let first = service.create_product(first).await.unwrap();
    let second = service
        .create_product(CreateProductInput {
            variants: vec![variant_input("Default", "F-2")],
            ..product_input("Blue Mug")
        })
        .await
        .unwrap();
    service.publish_product(&second.id).await.unwrap();

    let all = service
        .list_products(ProductFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(all.count, 2);

    let by_category = service
        .list_products(
            ProductFilter {
                category_id: Some(category.id.clone()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_category.count, 1);
    assert_eq!(by_category.data[0].id, first.id);

    let by_collection = service
        .list_products(
            ProductFilter {
                collection_id: Some(collection.id.clone()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_collection.count, 1);

    let by_tag = service
        .list_products(
            ProductFilter {
                tag_value: Some("featured".to_owned()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_tag.count, 1);

    let by_status = service
        .list_products(
            ProductFilter {
                status: Some(ProductStatus::Active),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_status.count, 1);
    assert_eq!(by_status.data[0].id, second.id);

    let by_text = service
        .list_products(
            ProductFilter {
                query: Some("trainer".to_owned()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_text.count, 1);

    let by_sku = service
        .list_products(
            ProductFilter {
                query: Some("F-2".to_owned()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(by_sku.count, 1);

    let page = service
        .list_products(
            ProductFilter::default(),
            Pagination::new(Some(1), Some(1)).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);
}

#[tokio::test]
async fn rejects_invalid_pagination() {
    let service = service().await;
    assert!(matches!(
        service
            .list_products(
                ProductFilter::default(),
                Pagination {
                    limit: 0,
                    offset: 0
                }
            )
            .await
            .unwrap_err(),
        AppError::Validation(_)
    ));
}

#[tokio::test]
async fn creates_updates_and_finds_variants() {
    let service = service().await;
    let product = basic_product(&service, "Variant Product", "V-1").await;

    let variant = service
        .create_variant(
            &product.id,
            CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("V-2".to_owned()),
                weight: Some(1.5),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(variant.product_id, product.id);
    assert_eq!(
        service.get_variant_by_sku("V-2").await.unwrap().id,
        variant.id
    );

    let updated = service
        .update_variant(
            &variant.id,
            UpdateVariantInput {
                title: Some("Extra Large".to_owned()),
                allow_backorder: Some(true),
                barcode: Some("12345".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Extra Large");
    assert!(updated.allow_backorder);
    assert_eq!(updated.barcode.as_deref(), Some("12345"));

    let reloaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(reloaded.variants.len(), 2);
}

#[tokio::test]
async fn variant_sku_updates_are_unique() {
    let service = service().await;
    let first = basic_product(&service, "One", "UNIQ-1").await;
    let second = basic_product(&service, "Two", "UNIQ-2").await;
    let variant = &second.variants[0];

    let error = service
        .update_variant(
            &variant.id,
            UpdateVariantInput {
                sku: Some("UNIQ-1".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
    assert_eq!(first.variants[0].sku.as_deref(), Some("UNIQ-1"));
}

#[tokio::test]
async fn unreferenced_variants_are_hard_deleted() {
    let (service, db) = service_with_db().await;
    let product = basic_product(&service, "Delete Variant", "DV-1").await;
    let variant_id = product.variants[0].id.clone();

    service.delete_variant(&variant_id).await.unwrap();
    assert!(
        service
            .get_variant(&variant_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_variants WHERE id = ?")
        .bind(&variant_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn referenced_variants_are_soft_deleted() {
    let (service, db) = service_with_db().await;
    let product = basic_product(&service, "Referenced Variant", "RV-1").await;
    let variant_id = product.variants[0].id.clone();

    sqlx::query("INSERT INTO carts (id, currency_code, created_at, updated_at) VALUES ('cart_1', 'USD', 0, 0)")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO cart_items \
         (id, cart_id, product_id, variant_id, quantity, title, unit_price, currency_code, \
          created_at, updated_at) VALUES ('item_1', 'cart_1', ?, ?, 1, 'Item', 100, 'USD', 0, 0)",
    )
    .bind(&product.id)
    .bind(&variant_id)
    .execute(db.pool())
    .await
    .unwrap();

    service.delete_variant(&variant_id).await.unwrap();
    assert!(
        service
            .get_variant(&variant_id)
            .await
            .unwrap_err()
            .is_not_found()
    );
    let deleted_at: Option<i64> =
        sqlx::query_scalar("SELECT deleted_at FROM product_variants WHERE id = ?")
            .bind(&variant_id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(deleted_at.is_some());
}

#[tokio::test]
async fn creates_and_lists_category_hierarchies() {
    let service = service().await;
    let parent = service
        .create_category(CreateCategoryInput {
            title: "Apparel".to_owned(),
            description: Some("Clothing".to_owned()),
            ..Default::default()
        })
        .await
        .unwrap();
    let child = service
        .create_category(CreateCategoryInput {
            title: "Shirts".to_owned(),
            parent_id: Some(parent.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(child.parent_id.as_deref(), Some(parent.id.as_str()));
    assert!(parent.is_active);
    assert_eq!(parent.handle, "apparel");

    let children = service
        .list_categories(Some(&parent.id), Pagination::default())
        .await
        .unwrap();
    assert_eq!(children.count, 1);
    assert_eq!(children.data[0].id, child.id);

    let all = service
        .list_categories(None, Pagination::default())
        .await
        .unwrap();
    assert_eq!(all.count, 2);

    let updated = service
        .update_category(
            &child.id,
            UpdateCategoryInput {
                title: Some("Tops".to_owned()),
                is_internal: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Tops");
    assert!(updated.is_internal);
    assert_eq!(updated.handle, "shirts");
}

#[tokio::test]
async fn category_cycles_and_duplicate_handles_are_rejected() {
    let service = service().await;
    let parent = service
        .create_category(CreateCategoryInput {
            title: "Parent".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let child = service
        .create_category(CreateCategoryInput {
            title: "Child".to_owned(),
            parent_id: Some(parent.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    let error = service
        .update_category(
            &parent.id,
            UpdateCategoryInput {
                parent_id: Some(child.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let error = service
        .update_category(
            &parent.id,
            UpdateCategoryInput {
                parent_id: Some(parent.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let error = service
        .create_category(CreateCategoryInput {
            title: "Duplicate".to_owned(),
            handle: Some("parent".to_owned()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));
}

#[tokio::test]
async fn category_deletion_validates_children_and_links() {
    let service = service().await;
    let parent = service
        .create_category(CreateCategoryInput {
            title: "Parent".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let child = service
        .create_category(CreateCategoryInput {
            title: "Child".to_owned(),
            parent_id: Some(parent.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(matches!(
        service.delete_category(&parent.id).await.unwrap_err(),
        AppError::Conflict(_)
    ));

    service.delete_category(&child.id).await.unwrap();
    assert!(
        service
            .get_category(&child.id)
            .await
            .unwrap_err()
            .is_not_found()
    );

    let linked = service
        .create_product(CreateProductInput {
            category_ids: vec![parent.id.clone()],
            ..product_input("Linked")
        })
        .await
        .unwrap();
    assert!(matches!(
        service.delete_category(&parent.id).await.unwrap_err(),
        AppError::Conflict(_)
    ));

    service
        .update_product(
            &linked.id,
            UpdateProductInput {
                category_ids: Some(Vec::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    service.delete_category(&parent.id).await.unwrap();
    assert!(
        service
            .get_category(&parent.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn collections_support_publish_and_product_links() {
    let (service, db) = service_with_db().await;
    let collection = service
        .create_collection(CreateCollectionInput {
            title: "Featured".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!collection.is_published);

    let published = service.publish_collection(&collection.id).await.unwrap();
    assert!(published.is_published);
    assert!(
        !service
            .unpublish_collection(&collection.id)
            .await
            .unwrap()
            .is_published
    );
    assert!(
        service
            .publish_collection(&collection.id)
            .await
            .unwrap()
            .is_published
    );

    let product = basic_product(&service, "Featured Product", "FP-1").await;
    service
        .add_product_to_collection(&product.id, &collection.id, 5)
        .await
        .unwrap();
    let linked = service.get_product(&product.id).await.unwrap();
    assert_eq!(linked.collections.len(), 1);
    assert_eq!(linked.collections[0].id, collection.id);

    let rank: i64 = sqlx::query_scalar(
        "SELECT rank FROM product_collection_links WHERE product_id = ? AND collection_id = ?",
    )
    .bind(&product.id)
    .bind(&collection.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(rank, 5);

    service
        .remove_product_from_collection(&product.id, &collection.id)
        .await
        .unwrap();
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .collections
            .is_empty()
    );
    assert!(
        service
            .remove_product_from_collection(&product.id, &collection.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn updates_collections() {
    let service = service().await;
    let collection = service
        .create_collection(CreateCollectionInput {
            title: "Old".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let updated = service
        .update_collection(
            &collection.id,
            UpdateCollectionInput {
                title: Some("New".to_owned()),
                description: Some("Curated".to_owned()),
                is_published: Some(true),
                rank: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "New");
    assert_eq!(updated.description.as_deref(), Some("Curated"));
    assert!(updated.is_published);
    assert_eq!(updated.rank, 2);
    assert_eq!(
        service
            .list_collections(Pagination::default())
            .await
            .unwrap()
            .count,
        1
    );
}

#[tokio::test]
async fn tags_can_be_attached_and_detached() {
    let service = service().await;
    let product = basic_product(&service, "Tagged", "TAG-1").await;

    let tag = service.attach_tag(&product.id, "new").await.unwrap();
    assert_eq!(tag.value, "new");
    assert_eq!(
        service.attach_tag(&product.id, "new").await.unwrap().id,
        tag.id
    );
    assert_eq!(
        service
            .list_tags(Pagination::default())
            .await
            .unwrap()
            .count,
        1
    );
    assert_eq!(service.get_tag_by_value("new").await.unwrap().id, tag.id);
    assert_eq!(
        service.get_product(&product.id).await.unwrap().tags.len(),
        1
    );

    service.detach_tag(&product.id, &tag.id).await.unwrap();
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .tags
            .is_empty()
    );
    assert!(
        service
            .detach_tag(&product.id, &tag.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn empty_tag_values_are_rejected() {
    let service = service().await;
    assert!(matches!(
        service.create_tag("   ").await.unwrap_err(),
        AppError::Validation(_)
    ));
}

#[tokio::test]
async fn add_remove_and_reorder_images() {
    let service = service().await;
    let product = service
        .create_product(CreateProductInput {
            images: vec![
                "https://example.com/a.jpg".to_owned(),
                "https://example.com/b.jpg".to_owned(),
            ],
            ..product_input("Imaged")
        })
        .await
        .unwrap();
    assert_eq!(product.images.len(), 2);
    assert_eq!(product.images[0].url, "https://example.com/a.jpg");

    let added = service
        .add_image(
            &product.id,
            CreateImageInput {
                url: "https://example.com/c.jpg".to_owned(),
                alt: Some("Third".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(added.alt.as_deref(), Some("Third"));

    let images = service.list_images(&product.id).await.unwrap();
    assert_eq!(images.len(), 3);
    let reordered = service
        .reorder_images(
            &product.id,
            &[added.id.clone(), images[0].id.clone(), images[1].id.clone()],
        )
        .await
        .unwrap();
    assert_eq!(reordered[0].id, added.id);

    service.remove_image(&added.id).await.unwrap();
    assert_eq!(service.list_images(&product.id).await.unwrap().len(), 2);
    assert!(
        service
            .remove_image(&added.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn images_can_target_a_variant() {
    let service = service().await;
    let product = basic_product(&service, "Variant Image", "VI-1").await;
    let variant_id = product.variants[0].id.clone();
    let image = service
        .add_image(
            &product.id,
            CreateImageInput {
                url: "https://example.com/variant.jpg".to_owned(),
                variant_id: Some(variant_id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(image.variant_id.as_deref(), Some(variant_id.as_str()));
    assert_eq!(service.get_image(&image.id).await.unwrap().id, image.id);
}

#[tokio::test]
async fn full_text_search_respects_publication_state() {
    let service = service().await;
    let shirt = published_product(&service, "Red Running Shirt", "FTS-1").await;
    let draft = basic_product(&service, "Draft Running Shirt", "FTS-2").await;
    let _mug = published_product(&service, "Blue Mug", "FTS-3").await;

    let results = service
        .search_products("running shirt", Pagination::default())
        .await
        .unwrap();
    assert_eq!(results.count, 1);
    assert_eq!(results.data[0].id, shirt.id);

    let admin = service
        .search_products_admin("running shirt", Pagination::default())
        .await
        .unwrap();
    assert_eq!(admin.count, 2);
    assert!(admin.data.iter().any(|product| product.id == draft.id));

    let by_sku = service
        .search_products("FTS-3", Pagination::default())
        .await
        .unwrap();
    assert_eq!(by_sku.count, 1);
}

#[tokio::test]
async fn blank_search_returns_an_empty_page() {
    let service = service().await;
    let results = service
        .search_products("   ", Pagination::default())
        .await
        .unwrap();
    assert_eq!(results.count, 0);
    assert!(results.data.is_empty());
}

#[tokio::test]
async fn search_reindex_rebuilds_the_fts_index() {
    let (service, db) = service_with_db().await;
    published_product(&service, "Rebuild Me", "RE-1").await;
    published_product(&service, "Another Product", "RE-2").await;

    sqlx::query("DELETE FROM product_search")
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        service
            .search_products("rebuild", Pagination::default())
            .await
            .unwrap()
            .count,
        0
    );

    let indexed = service.reindex_search().await.unwrap();
    assert_eq!(indexed, 2);
    assert_eq!(
        service
            .search_products("rebuild", Pagination::default())
            .await
            .unwrap()
            .count,
        1
    );
}

#[tokio::test]
async fn search_index_tracks_updates_deletes_and_variants() {
    let service = service().await;
    let product = published_product(&service, "Original Widget", "SI-1").await;

    service
        .update_product(
            &product.id,
            UpdateProductInput {
                title: Some("Renamed Gadget".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .search_products("widget", Pagination::default())
            .await
            .unwrap()
            .count,
        0
    );
    assert_eq!(
        service
            .search_products("gadget", Pagination::default())
            .await
            .unwrap()
            .count,
        1
    );

    service
        .create_variant(
            &product.id,
            CreateVariantInput {
                title: "Large".to_owned(),
                sku: Some("SI-NEW".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .search_products("SI-NEW", Pagination::default())
            .await
            .unwrap()
            .count,
        1
    );

    service.delete_product(&product.id).await.unwrap();
    assert_eq!(
        service
            .search_products("gadget", Pagination::default())
            .await
            .unwrap()
            .count,
        0
    );
}

#[tokio::test]
async fn metadata_size_is_validated_for_products() {
    let service = service().await;
    let mut metadata = Metadata::empty();
    metadata.insert("blob", serde_json::json!("x".repeat(20_000)));
    let error = service
        .create_product(CreateProductInput {
            metadata,
            ..product_input("Large Metadata")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn negative_dimensions_are_rejected() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            dimensions: Some(Dimensions {
                length: Some(-1.0),
                ..Default::default()
            }),
            ..product_input("Bad Dimensions")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn variant_title_is_required() {
    let service = service().await;
    let error = service
        .create_product(CreateProductInput {
            variants: vec![CreateVariantInput {
                title: "  ".to_owned(),
                ..Default::default()
            }],
            ..product_input("Bad Variant")
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn updating_missing_products_and_variants_fails() {
    let service = service().await;
    assert!(
        service
            .update_product("prod_missing", UpdateProductInput::default())
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .update_variant("var_missing", UpdateVariantInput::default())
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert!(
        service
            .get_variant_by_sku("MISSING")
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn links_to_missing_entities_are_rejected() {
    let service = service().await;
    assert!(matches!(
        service
            .create_product(CreateProductInput {
                category_ids: vec!["cat_missing".to_owned()],
                ..product_input("Orphan")
            })
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(matches!(
        service
            .create_product(CreateProductInput {
                collection_ids: vec!["coll_missing".to_owned()],
                ..product_input("Orphan")
            })
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
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

#[tokio::test]
async fn sets_and_replaces_product_sales_channels() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_pos").await;
    insert_sales_channel(&db, "sc_web").await;
    let product = basic_product(&service, "Channeled", "CH-1").await;
    assert!(product.sales_channel_ids.is_empty());

    service
        .set_product_sales_channels(
            &product.id,
            &[
                "sc_web".to_owned(),
                "sc_pos".to_owned(),
                "sc_web".to_owned(),
            ],
        )
        .await
        .unwrap();
    let loaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(
        loaded.sales_channel_ids,
        vec!["sc_pos".to_owned(), "sc_web".to_owned()]
    );
    assert_eq!(
        service
            .list_product_sales_channel_ids(&product.id)
            .await
            .unwrap(),
        vec!["sc_pos".to_owned(), "sc_web".to_owned()]
    );

    service
        .set_product_sales_channels(&product.id, &["sc_pos".to_owned()])
        .await
        .unwrap();
    assert_eq!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids,
        vec!["sc_pos".to_owned()]
    );

    service
        .set_product_sales_channels(&product.id, &[])
        .await
        .unwrap();
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids
            .is_empty()
    );
}

#[tokio::test]
async fn rejects_unknown_sales_channels() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_web").await;
    let product = basic_product(&service, "Channeled", "CH-2").await;

    assert!(matches!(
        service
            .set_product_sales_channels(
                &product.id,
                &["sc_web".to_owned(), "sc_missing".to_owned()]
            )
            .await
            .unwrap_err(),
        AppError::NotFound(_)
    ));
    assert!(
        service
            .get_product(&product.id)
            .await
            .unwrap()
            .sales_channel_ids
            .is_empty()
    );
    assert!(
        service
            .set_product_sales_channels("prod_missing", &["sc_web".to_owned()])
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn filters_and_searches_products_by_sales_channel() {
    let (service, db) = service_with_db().await;
    insert_sales_channel(&db, "sc_pos").await;
    insert_sales_channel(&db, "sc_web").await;

    let web = service
        .create_product(CreateProductInput {
            title: "Red Shirt".to_owned(),
            variants: vec![variant_input("Default", "SC-1")],
            ..Default::default()
        })
        .await
        .unwrap();
    let pos = service
        .create_product(CreateProductInput {
            title: "Blue Shirt".to_owned(),
            variants: vec![variant_input("Default", "SC-2")],
            ..Default::default()
        })
        .await
        .unwrap();
    service
        .set_product_sales_channels(&web.id, &["sc_web".to_owned()])
        .await
        .unwrap();
    service
        .set_product_sales_channels(&pos.id, &["sc_pos".to_owned()])
        .await
        .unwrap();
    service.publish_product(&web.id).await.unwrap();
    service.publish_product(&pos.id).await.unwrap();

    let web_only = service
        .list_products(
            ProductFilter {
                sales_channel_ids: Some(vec!["sc_web".to_owned()]),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(web_only.count, 1);
    assert_eq!(web_only.data[0].id, web.id);
    assert_eq!(
        web_only.data[0].sales_channel_ids,
        vec!["sc_web".to_owned()]
    );

    let no_filter = service
        .list_products(
            ProductFilter {
                sales_channel_ids: Some(Vec::new()),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(no_filter.count, 2);

    let storefront = service
        .search_products_with_filter(
            "shirt",
            ProductFilter {
                status: Some(ProductStatus::Active),
                sales_channel_ids: Some(vec!["sc_pos".to_owned()]),
                ..Default::default()
            },
            Pagination::default(),
        )
        .await
        .unwrap();
    assert_eq!(storefront.count, 1);
    assert_eq!(storefront.data[0].id, pos.id);

    assert!(
        service
            .get_published_by_handle_in_channels(&web.handle, &["sc_pos".to_owned()])
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert_eq!(
        service
            .get_published_by_handle_in_channels(&web.handle, &["sc_web".to_owned()])
            .await
            .unwrap()
            .id,
        web.id
    );
    assert_eq!(
        service
            .get_published_by_handle_in_channels(&web.handle, &[])
            .await
            .unwrap()
            .id,
        web.id
    );
}
