use super::*;

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
