//! Inventory service tests.

use std::sync::Arc;

use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{
    AppError, CheckoutId, InventoryItemId, InventoryLocationId, LineItemId, Metadata, OrderId,
    Pagination, ReturnId, StoreId, VariantId, now_timestamp,
};

use crate::domain::{
    CreateInventoryItemInput, CreateLocationInput, ExchangeRestockLine, OrderEditStockLine,
    Reference, ReservationLine, ReservationStatus, ReturnRestockLine, StockConsumption,
    exchange_restock_reference, order_edit_stock_reference, return_restock_reference,
};
use crate::service::InventoryService;

async fn setup() -> (StoreDatabase, InventoryService) {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    let service = InventoryService::new(db.clone());
    (db, service)
}

async fn location(service: &InventoryService, name: &str) -> InventoryLocationId {
    let created = service
        .create_location(CreateLocationInput {
            name: name.to_owned(),
            address: None,
            metadata: None,
        })
        .await
        .unwrap();
    created.id
}

async fn item_with_variant(service: &InventoryService, variant: &VariantId) -> InventoryItemId {
    let created = service
        .create_item(CreateInventoryItemInput {
            sku: Some(format!("sku-{}", variant.as_str())),
            variant_id: Some(variant.clone()),
            metadata: None,
        })
        .await
        .unwrap();
    created.id
}

mod catalog;
mod exchanges;
mod fulfillments;
mod order_edits;
mod reservations;
mod returns;
mod stock;
