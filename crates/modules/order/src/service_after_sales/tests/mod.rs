use fastcart_infrastructure::StoreDatabase;
use fastcart_kernel::{OrderId, Pagination, PaymentId, RefundId, StoreId, VariantId};
use serde_json::json;

use crate::domain::{
    CreateOrderInput, CreateOrderItemInput, CreateOrderTaxLineInput, Order, OrderStatus,
    RefundInput,
};

use crate::after_sales::{
    ClaimItemInput, ClaimItemType, ClaimStatus, CreateClaimInput, CreateExchangeInput,
    CreateOrderEditInput, CreateReturnInput, ExchangeItemInput, ExchangeStatus, OrderEditAction,
    OrderEditItemInput, OrderEditStatus, OrderReturn, ReceiveReturnInput, ReceiveReturnItemInput,
    ReturnItemInput, ReturnStatus, StoreReturnFilter, WriteOffReturnInput,
};

use crate::service::OrderService;
/// A single item-scoped tax line, matching what checkout snapshots.
fn tax_line(amount: i64) -> CreateOrderTaxLineInput {
    CreateOrderTaxLineInput {
        item_index: Some(0),
        shipping_option_id: None,
        name: "Sales tax".to_owned(),
        rate_bps: 1_000,
        amount,
        is_tax_inclusive: false,
    }
}

async fn service() -> OrderService {
    let db = StoreDatabase::in_memory(StoreId::new()).await.unwrap();
    OrderService::new(db)
}

async fn order(service: &OrderService, quantity: i64) -> Order {
    let unit_price = 1_000;
    service
        .create_order(CreateOrderInput {
            email: "buyer@example.com".to_owned(),
            currency_code: "USD".to_owned(),
            items: vec![CreateOrderItemInput {
                product_title: "Widget".to_owned(),
                unit_price,
                quantity,
                ..CreateOrderItemInput::default()
            }],
            subtotal: unit_price * quantity,
            total: unit_price * quantity,
            shipping_address: json!({}),
            billing_address: json!({}),
            ..CreateOrderInput::default()
        })
        .await
        .unwrap()
}

fn return_input(order: &Order, quantity: i64) -> CreateReturnInput {
    CreateReturnInput {
        reason: Some("damaged".to_owned()),
        reason_id: None,
        refund_id: None,
        metadata: fastcart_kernel::Metadata::default(),
        items: vec![ReturnItemInput {
            order_item_id: order.items[0].id.clone(),
            quantity,
            reason: Some("broken".to_owned()),
        }],
    }
}

mod claims;
mod customer_returns;
mod edits;
mod errors;
mod exchanges;
mod refunds;
mod return_reasons;
mod returns;
