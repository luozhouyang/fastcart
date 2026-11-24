use super::*;

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
async fn reordering_images_rejects_duplicates_and_foreign_ids() {
    let service = service().await;
    let first = service
        .create_product(CreateProductInput {
            images: vec![
                "https://example.com/a.jpg".to_owned(),
                "https://example.com/b.jpg".to_owned(),
            ],
            ..product_input("First")
        })
        .await
        .unwrap();
    let second = service
        .create_product(CreateProductInput {
            images: vec!["https://example.com/c.jpg".to_owned()],
            ..product_input("Second")
        })
        .await
        .unwrap();

    let first_id = first.images[0].id.clone();
    let error = service
        .reorder_images(&first.id, &[first_id.clone(), first_id])
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));

    let error = service
        .reorder_images(&first.id, &[second.images[0].id.clone()])
        .await
        .unwrap_err();
    assert!(error.is_not_found());
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
