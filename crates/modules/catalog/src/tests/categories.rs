use super::*;

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
