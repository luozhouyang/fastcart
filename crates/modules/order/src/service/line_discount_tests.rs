use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::StoreId;
use serde_json::json;

use super::OrderService;
use crate::domain::{CreateOrderInput, CreateOrderItemInput};

async fn service() -> OrderService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    OrderService::new(db)
}

/// A line cannot be discounted past its own value: the remainder would make
/// the line total negative, and every total derived from it — including the
/// amount the customer was charged — stops meaning anything.
#[tokio::test]
async fn rejects_a_line_discounted_past_its_own_value() {
    let service = service().await;
    let error = service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price: 1_000,
                quantity: 1,
                subtotal: Some(1_000),
                discount_total: Some(1_500),
                tax_total: Some(0),
                total: Some(0),
                requires_shipping: true,
                ..CreateOrderItemInput::default()
            }],
            subtotal: 1_000,
            discount_total: 1_500,
            total: 0,
            shipping_address: json!({"city": "Berlin"}),
            billing_address: json!({"city": "Berlin"}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap_err();

    assert!(matches!(error, fastcart_kernel::AppError::Validation(_)));
}
