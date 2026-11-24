use super::*;

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
