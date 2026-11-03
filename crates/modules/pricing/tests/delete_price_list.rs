//! Price list deletion integration test.

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{AppError, Metadata, StoreId};
use fastcart_module_pricing::{
    CreatePriceListInput, PriceListStatus, PriceListType, PricingService,
};

#[tokio::test]
async fn deleting_a_price_list_removes_it() {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = PricingService::new(db);
    let list = service
        .create_price_list(CreatePriceListInput {
            title: "Summer".to_owned(),
            description: None,
            price_type: PriceListType::Sale,
            status: PriceListStatus::Active,
            starts_at: None,
            ends_at: None,
            metadata: Metadata::empty(),
            rules: Vec::new(),
        })
        .await
        .unwrap();

    service.delete_price_list(&list.id).await.unwrap();
    let error = service.get_price_list(&list.id).await.unwrap_err();
    assert!(matches!(error, AppError::NotFound(_)));
    assert!(service.delete_price_list(&list.id).await.is_err());
}
