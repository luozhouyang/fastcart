use super::*;

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
