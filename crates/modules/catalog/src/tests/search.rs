use super::*;

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
