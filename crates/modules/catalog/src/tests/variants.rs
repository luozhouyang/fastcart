use super::*;

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
