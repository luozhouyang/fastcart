use super::*;

#[tokio::test]
async fn locations_crud_and_pagination() {
    let (_db, service) = setup().await;
    let first = service
        .create_location(CreateLocationInput {
            name: "Main".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    let second = service
        .create_location(CreateLocationInput {
            name: "Overflow".to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();

    let page = service
        .list_locations(Pagination {
            limit: 1,
            offset: 0,
        })
        .await
        .unwrap();
    assert_eq!(page.count, 2);
    assert_eq!(page.data.len(), 1);

    let updated = service
        .update_location(
            &first.id,
            crate::domain::UpdateLocationInput {
                name: Some("Main warehouse".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Main warehouse");
    assert_eq!(
        service.get_location(&second.id).await.unwrap().name,
        "Overflow"
    );

    assert!(matches!(
        service.get_location(&InventoryLocationId::new()).await,
        Err(AppError::NotFound(_))
    ));
}

#[tokio::test]
async fn item_creation_enforces_unique_variant_and_sku() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let item = service
        .create_item(CreateInventoryItemInput {
            sku: Some("abc".to_owned()),
            variant_id: Some(variant.clone()),
            metadata: Some(Metadata::empty()),
        })
        .await
        .unwrap();

    assert_eq!(
        service.get_item_by_variant(&variant).await.unwrap().id,
        item.id
    );
    assert_eq!(
        service.get_item(&item.id).await.unwrap().sku.as_deref(),
        Some("abc")
    );

    assert!(matches!(
        service
            .create_item(CreateInventoryItemInput {
                sku: Some("def".to_owned()),
                variant_id: Some(variant.clone()),
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));
    assert!(matches!(
        service
            .create_item(CreateInventoryItemInput {
                sku: Some("abc".to_owned()),
                variant_id: Some(VariantId::new()),
                metadata: None,
            })
            .await,
        Err(AppError::Conflict(_))
    ));

    let other = service
        .create_item(CreateInventoryItemInput {
            sku: None,
            variant_id: None,
            metadata: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        service.attach_variant(&other.id, &variant).await,
        Err(AppError::Conflict(_))
    ));
    let attached = service
        .attach_variant(&other.id, &VariantId::new())
        .await
        .unwrap();
    assert!(attached.variant_id.is_some());
}

#[tokio::test]
async fn availability_sums_levels_and_ignores_unknown_variants() {
    let (_db, service) = setup().await;
    let first = location(&service, "A").await;
    let second = location(&service, "B").await;
    let variant = VariantId::new();
    let item = item_with_variant(&service, &variant).await;
    service
        .adjust_stock(&item, &first, 3, "receiving", None)
        .await
        .unwrap();
    service
        .adjust_stock(&item, &second, 6, "receiving", None)
        .await
        .unwrap();

    assert_eq!(service.variant_availability(&variant).await.unwrap(), 9);
    assert_eq!(service.list_levels(&item).await.unwrap().len(), 2);
    assert_eq!(
        service
            .list_levels_for_location(&first)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .variant_availability(&VariantId::new())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        service
            .available_quantity(&item, &InventoryLocationId::new())
            .await
            .unwrap_err()
            .code(),
        "NOT_FOUND"
    );
}

#[tokio::test]
async fn concurrent_item_creation_maps_unique_violations_to_conflict() {
    let (_db, service) = setup().await;
    let variant = VariantId::new();
    let input = CreateInventoryItemInput {
        sku: Some("race-sku".to_owned()),
        variant_id: Some(variant),
        metadata: None,
    };
    let (first, second) = tokio::join!(
        service.create_item(input.clone()),
        service.create_item(input),
    );

    let successes = usize::from(first.is_ok()) + usize::from(second.is_ok());
    assert_eq!(
        successes, 1,
        "exactly one concurrent creation should win: {first:?} / {second:?}"
    );
    assert!(
        matches!(&first, Err(AppError::Conflict(_)))
            || matches!(&second, Err(AppError::Conflict(_))),
        "the loser must surface a conflict: {first:?} / {second:?}"
    );
    assert!(!matches!(&first, Err(AppError::Internal(_))));
    assert!(!matches!(&second, Err(AppError::Internal(_))));
}

#[tokio::test]
async fn concurrent_variant_attach_maps_unique_violations_to_conflict() {
    let (_db, service) = setup().await;
    let first = service
        .create_item(CreateInventoryItemInput::default())
        .await
        .unwrap();
    let second = service
        .create_item(CreateInventoryItemInput::default())
        .await
        .unwrap();
    let variant = VariantId::new();
    let (left, right) = tokio::join!(
        service.attach_variant(&first.id, &variant),
        service.attach_variant(&second.id, &variant),
    );

    let successes = usize::from(left.is_ok()) + usize::from(right.is_ok());
    assert_eq!(
        successes, 1,
        "exactly one concurrent attach should win: {left:?} / {right:?}"
    );
    assert!(
        matches!(&left, Err(AppError::Conflict(_))) || matches!(&right, Err(AppError::Conflict(_))),
        "the loser must surface a conflict: {left:?} / {right:?}"
    );
    assert!(!matches!(&left, Err(AppError::Internal(_))));
    assert!(!matches!(&right, Err(AppError::Internal(_))));
}
