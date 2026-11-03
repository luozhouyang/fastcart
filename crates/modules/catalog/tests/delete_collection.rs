//! Collection deletion integration test.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, StoreId};
use fastcart_module_catalog::{CatalogService, CreateCollectionInput};

#[tokio::test]
async fn deleting_a_collection_soft_deletes_it() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = CatalogService::new(db);
    let collection = service
        .create_collection(CreateCollectionInput {
            title: "Summer".to_owned(),
            handle: Some("summer".to_owned()),
            description: None,
            is_published: None,
            rank: None,
            metadata: Default::default(),
        })
        .await
        .unwrap();

    service
        .delete_collection(collection.id.as_str())
        .await
        .unwrap();
    let error = service
        .get_collection(collection.id.as_str())
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
    assert!(
        service
            .delete_collection(collection.id.as_str())
            .await
            .is_err()
    );
}
