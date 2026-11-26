use super::*;

#[tokio::test]
async fn product_types_support_crud_and_listing() {
    let service = service().await;
    let mut metadata = Metadata::empty();
    metadata.insert("kind", serde_json::json!("apparel"));

    let shirt = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Shirt".to_owned(),
            metadata,
        })
        .await
        .unwrap();
    assert!(shirt.id.starts_with("ptype_"));
    assert_eq!(shirt.value, "Shirt");
    assert_eq!(
        shirt.metadata.get("kind"),
        Some(&serde_json::json!("apparel"))
    );
    assert!(shirt.created_at > 0);

    let loaded = service.get_product_type(&shirt.id).await.unwrap();
    assert_eq!(loaded, shirt);

    let other = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Digital".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let page = service
        .list_product_types(Pagination::default())
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data[0].id, other.id);
    assert_eq!(page.data[1].id, shirt.id);

    let updated = service
        .update_product_type(
            &shirt.id,
            crate::UpdateProductTypeInput {
                value: Some("T-Shirt".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.value, "T-Shirt");
    assert_eq!(updated.metadata, shirt.metadata);

    service.delete_product_type(&other.id).await.unwrap();
    assert!(
        service
            .get_product_type(&other.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn duplicate_product_type_values_are_rejected() {
    let service = service().await;
    service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Shirt".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let error = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "  shirt ".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let digital = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Digital".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let error = service
        .update_product_type(
            &digital.id,
            crate::UpdateProductTypeInput {
                value: Some("SHIRT".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Conflict(_)));

    let error = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "   ".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let long = "a".repeat(crate::MAX_PRODUCT_TYPE_VALUE_LENGTH + 1);
    let error = service
        .create_product_type(crate::CreateProductTypeInput {
            value: long,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn products_assign_and_validate_product_types() {
    let service = service().await;
    let product_type = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Shirt".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let product = service
        .create_product(CreateProductInput {
            product_type_id: Some(product_type.id.clone()),
            ..product_input("Typed")
        })
        .await
        .unwrap();
    assert_eq!(
        product.product_type_id.as_deref(),
        Some(product_type.id.as_str())
    );

    let loaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(loaded.product_type_id, Some(product_type.id.clone()));

    let listed = service
        .list_products(ProductFilter::default(), Pagination::default())
        .await
        .unwrap();
    assert_eq!(
        listed.data[0].product_type_id,
        Some(product_type.id.clone())
    );

    let cleared = service
        .update_product(
            &product.id,
            UpdateProductInput {
                product_type_id: Some(String::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(cleared.product_type_id.is_none());

    let reassigned = service
        .update_product(
            &product.id,
            UpdateProductInput {
                product_type_id: Some(product_type.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        reassigned.product_type_id.as_deref(),
        Some(product_type.id.as_str())
    );

    let missing = service
        .create_product(CreateProductInput {
            product_type_id: Some("ptype_missing".to_owned()),
            ..product_input("Invalid type")
        })
        .await
        .unwrap_err();
    assert!(missing.is_not_found());

    let missing = service
        .update_product(
            &product.id,
            UpdateProductInput {
                product_type_id: Some("ptype_missing".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(missing.is_not_found());

    // The failed update left the existing assignment intact.
    let loaded = service.get_product(&product.id).await.unwrap();
    assert_eq!(
        loaded.product_type_id.as_deref(),
        Some(product_type.id.as_str())
    );
}

#[tokio::test]
async fn deleting_a_product_type_clears_product_assignments() {
    let service = service().await;
    let product_type = service
        .create_product_type(crate::CreateProductTypeInput {
            value: "Shirt".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();
    let first = service
        .create_product(CreateProductInput {
            product_type_id: Some(product_type.id.clone()),
            ..product_input("First")
        })
        .await
        .unwrap();
    let second = service
        .create_product(CreateProductInput {
            product_type_id: Some(product_type.id.clone()),
            ..product_input("Second")
        })
        .await
        .unwrap();
    let untagged = service
        .create_product(product_input("Untagged"))
        .await
        .unwrap();

    service.delete_product_type(&product_type.id).await.unwrap();

    for product_id in [&first.id, &second.id] {
        let product = service.get_product(product_id).await.unwrap();
        assert!(product.product_type_id.is_none());
    }
    let product = service.get_product(&untagged.id).await.unwrap();
    assert!(product.product_type_id.is_none());

    assert!(
        service
            .delete_product_type(&product_type.id)
            .await
            .unwrap_err()
            .is_not_found()
    );
}
