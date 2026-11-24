use super::*;

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
async fn updating_options_preserves_existing_variant_links() {
    let service = service().await;
    let product = service
        .create_product(CreateProductInput {
            title: "Shirt".to_owned(),
            options: vec![CreateOptionInput {
                title: "Size".to_owned(),
                values: vec!["S".to_owned(), "M".to_owned()],
                ..Default::default()
            }],
            variants: vec![CreateVariantInput {
                title: "Small".to_owned(),
                sku: Some("SHIRT-S".to_owned()),
                option_values: vec![VariantOptionInput {
                    option_title: "Size".to_owned(),
                    value: "S".to_owned(),
                }],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    let value_id = product.options[0].values[0].id.clone();
    assert_eq!(product.variants[0].option_value_ids, vec![value_id.clone()]);

    // Replacing the option set without resubmitting variants must keep the
    // existing option-value identifiers so the variant links stay valid.
    let updated = service
        .update_product(
            &product.id,
            UpdateProductInput {
                options: Some(vec![CreateOptionInput {
                    title: "Size".to_owned(),
                    values: vec!["S".to_owned(), "M".to_owned(), "L".to_owned()],
                    ..Default::default()
                }]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.options.len(), 1);
    assert_eq!(updated.options[0].values[0].id, value_id);
    assert_eq!(updated.variants.len(), 1);
    assert_eq!(updated.variants[0].option_value_ids, vec![value_id.clone()]);
    assert!(
        updated.options[0]
            .values
            .iter()
            .any(|value| value.id == value_id)
    );
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
