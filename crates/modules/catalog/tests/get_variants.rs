//! Batch variant loading must agree with loading variants one at a time.

use std::collections::HashMap;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use fastcart_module_catalog::{CatalogService, CreateProductInput, CreateVariantInput};

async fn service_with_variants() -> (CatalogService, Vec<String>) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CatalogService::new(db);
    let product = service
        .create_product(CreateProductInput {
            title: "T-Shirt".to_owned(),
            variants: vec![
                CreateVariantInput {
                    title: "Medium".to_owned(),
                    sku: Some("TSHIRT-M".to_owned()),
                    ..Default::default()
                },
                CreateVariantInput {
                    title: "Large".to_owned(),
                    sku: Some("TSHIRT-L".to_owned()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        })
        .await
        .unwrap();
    let ids = product
        .variants
        .iter()
        .map(|variant| variant.id.clone())
        .collect();
    (service, ids)
}

#[tokio::test]
async fn batch_loading_matches_loading_one_at_a_time() {
    let (service, ids) = service_with_variants().await;

    let batch = service.get_variants(&ids).await.unwrap();

    assert_eq!(batch.len(), 2);
    for id in &ids {
        let one = service.get_variant(id).await.unwrap();
        assert_eq!(
            batch.get(id),
            Some(&one),
            "batch and single loads must agree for {id}"
        );
    }
}

#[tokio::test]
async fn unknown_ids_are_absent_rather_than_an_error() {
    let (service, ids) = service_with_variants().await;
    let mut requested = ids.clone();
    requested.push("var_missing".to_owned());

    let batch: HashMap<String, _> = service.get_variants(&requested).await.unwrap();

    assert_eq!(batch.len(), 2);
    assert!(!batch.contains_key("var_missing"));
}

#[tokio::test]
async fn an_empty_request_makes_no_round_trip() {
    let (service, _) = service_with_variants().await;
    assert!(service.get_variants(&[]).await.unwrap().is_empty());
}
